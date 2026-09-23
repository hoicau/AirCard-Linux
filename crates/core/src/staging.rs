//! Transaction staging and StreamingZip layout adapted from AirCard-Windows v1.2.2 (MIT).
//! The generated link is the only intentional path escape; imported resource names stay relative.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagingPlan {
    pub transaction: String,
}
impl StagingPlan {
    pub fn validate(&self) -> Result<(), crate::Error> {
        validate_transaction(&self.transaction)?;
        Ok(())
    }
    pub fn source(&self) -> String {
        format!("AirCard-Linux-Theme-{}", self.transaction)
    }
    pub fn link(&self) -> String {
        format!("AirCard-Linux-Link-{}", self.transaction)
    }
    pub fn asset_id(&self) -> String {
        format!("../../{}/p0/p1/p2/link", self.source())
    }
    pub fn request(
        &self,
        snapshot: &crate::books::BooksSnapshot,
    ) -> Result<(Vec<u8>, std::collections::BTreeSet<String>), crate::Error> {
        self.validate()?;
        let temporary_id = format!("AirCard-Linux-Test-{}.epub", self.transaction);
        let (bytes, retained) = crate::books::preserving_books_plist(snapshot, &temporary_id)?;
        let mut request = crate::decode_binary(&bytes)?;
        let rows = request
            .as_dictionary_mut()
            .and_then(|d| d.get_mut("Books"))
            .and_then(plist::Value::as_array_mut)
            .ok_or(crate::Error::UnsafePath)?;
        let row = rows
            .last_mut()
            .and_then(plist::Value::as_dictionary_mut)
            .ok_or(crate::Error::UnsafePath)?;
        row.insert(
            "Persistent ID".into(),
            plist::Value::String(self.asset_id()),
        );
        Ok((crate::encode_binary(&request)?, retained))
    }
}
pub fn validate_transaction(id: &str) -> Result<(), crate::Error> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(crate::Error::UnsafePath);
    }
    Ok(())
}
pub fn is_link_transfer(id: &str, destination: &str) -> bool {
    id.strip_prefix("../../AirCard-Linux-Theme-")
        .and_then(|s| s.strip_suffix("/p0/p1/p2/link"))
        .is_some_and(|transaction| {
            validate_transaction(transaction).is_ok()
                && destination == format!("AirCard-Linux-Link-{transaction}")
        })
}
const SZ_EXTRA_ID: u16 = 0x5A53;

// Unix file mode constants
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

struct StoredZipEntry {
    name: String,
    mode: u32,
    data: Vec<u8>,
}

fn crc32_simple(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

pub(crate) fn build_streaming_zip_archive_multi(
    target: &str,
    items: &[(&str, &[u8])],
) -> Result<Vec<u8>, crate::Error> {
    let target_tail = target.strip_prefix('/').unwrap_or(target);

    let mut metadata_plist = Vec::new();
    let mut meta_dict = HashMap::new();
    meta_dict.insert("Version".to_string(), plist::Value::Integer(2.into()));
    plist::to_writer_binary(
        &mut metadata_plist,
        &plist::Value::Dictionary(meta_dict.into_iter().collect()),
    )?;

    let mut entries = Vec::new();

    // META-INF/ (dir 0755)
    entries.push(StoredZipEntry {
        name: "META-INF/".to_string(),
        mode: S_IFDIR | 0o755,
        data: Vec::new(),
    });

    // META-INF/com.apple.ZipMetadata.plist (reg 0600)
    entries.push(StoredZipEntry {
        name: "META-INF/com.apple.ZipMetadata.plist".to_string(),
        mode: S_IFREG | 0o600,
        data: metadata_plist,
    });

    // p0/, p0/p1/, p0/p1/p2/ (dir 0755)
    for dir in &["p0/", "p0/p1/", "p0/p1/p2/"] {
        entries.push(StoredZipEntry {
            name: dir.to_string(),
            mode: S_IFDIR | 0o755,
            data: Vec::new(),
        });
    }

    // p0/p1/p2/link (symlink 0777)
    let symlink_target = format!("../../../{}", target_tail);
    entries.push(StoredZipEntry {
        name: "p0/p1/p2/link".to_string(),
        mode: S_IFLNK | 0o777,
        data: symlink_target.into_bytes(),
    });

    // Intermediary directories of target_tail (0755)
    let mut cursor = String::new();
    for component in target_tail.split('/') {
        if component.is_empty() {
            continue;
        }
        cursor.push_str(component);
        cursor.push('/');
        entries.push(StoredZipEntry {
            name: cursor.clone(),
            mode: S_IFDIR | 0o755,
            data: Vec::new(),
        });
    }

    // Payloads (reg 0600)
    if items.len() == 1 && items[0].0 == "payload" {
        entries.push(StoredZipEntry {
            name: "payload".to_string(),
            mode: S_IFREG | 0o600,
            data: items[0].1.to_vec(),
        });
    } else {
        for (idx, (_leaf, payload)) in items.iter().enumerate() {
            entries.push(StoredZipEntry {
                name: format!("payload_{}", idx),
                mode: S_IFREG | 0o600,
                data: payload.to_vec(),
            });
        }
        if !items.is_empty() {
            entries.push(StoredZipEntry {
                name: "payload".to_string(),
                mode: S_IFREG | 0o600,
                data: items[0].1.to_vec(),
            });
        }
    }

    // Pack into stored zip with Apple StreamingZip Unix metadata
    let mut output = Vec::new();
    let mut cd_entries = Vec::new();

    for entry in entries {
        let offset = output.len() as u32;
        let crc = crc32_simple(&entry.data);
        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len() as u16;

        let extra_mode = (entry.mode & 0xFFFF) as u16;
        let mut extra = Vec::new();
        extra.extend_from_slice(&SZ_EXTRA_ID.to_le_bytes());
        extra.extend_from_slice(&2u16.to_le_bytes());
        extra.extend_from_slice(&extra_mode.to_le_bytes());
        let extra_len = extra.len() as u16;

        // Local header (0x04034b50)
        output.extend_from_slice(&0x04034b50u32.to_le_bytes());
        output.extend_from_slice(&20u16.to_le_bytes()); // version needed
        output.extend_from_slice(&0u16.to_le_bytes()); // flags
        output.extend_from_slice(&0u16.to_le_bytes()); // compression = stored (0)
        output.extend_from_slice(&0x2800u16.to_le_bytes()); // mod time
        output.extend_from_slice(&0x5D30u16.to_le_bytes()); // mod date
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&(entry.data.len() as u32).to_le_bytes()); // compressed size
        output.extend_from_slice(&(entry.data.len() as u32).to_le_bytes()); // uncompressed size
        output.extend_from_slice(&name_len.to_le_bytes());
        output.extend_from_slice(&extra_len.to_le_bytes());
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(&extra);
        output.extend_from_slice(&entry.data);

        cd_entries.push((
            entry.name,
            entry.mode,
            crc,
            entry.data.len() as u32,
            offset,
            extra,
        ));
    }

    let cd_start = output.len() as u32;
    for (name, mode, crc, len, offset, extra) in &cd_entries {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let extra_len = extra.len() as u16;
        let ext_attr = (mode & 0xFFFF) << 16;

        // Central directory header (0x02014b50)
        output.extend_from_slice(&0x02014b50u32.to_le_bytes());
        output.extend_from_slice(&((3u16 << 8) | 20u16).to_le_bytes()); // version made by = Unix (3), 2.0
        output.extend_from_slice(&20u16.to_le_bytes()); // version needed
        output.extend_from_slice(&0u16.to_le_bytes()); // flags
        output.extend_from_slice(&0u16.to_le_bytes()); // compression = 0
        output.extend_from_slice(&0x2800u16.to_le_bytes()); // time
        output.extend_from_slice(&0x5D30u16.to_le_bytes()); // date
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&len.to_le_bytes()); // compressed
        output.extend_from_slice(&len.to_le_bytes()); // uncompressed
        output.extend_from_slice(&name_len.to_le_bytes());
        output.extend_from_slice(&extra_len.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes()); // comment len
        output.extend_from_slice(&0u16.to_le_bytes()); // disk start
        output.extend_from_slice(&0u16.to_le_bytes()); // internal attr
        output.extend_from_slice(&ext_attr.to_le_bytes()); // external attr
        output.extend_from_slice(&offset.to_le_bytes());
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(extra);
    }

    let cd_len = (output.len() as u32) - cd_start;
    let entry_count = cd_entries.len() as u16;

    // End of central directory record (0x06054b50)
    output.extend_from_slice(&0x06054b50u32.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // disk num
    output.extend_from_slice(&0u16.to_le_bytes()); // start disk
    output.extend_from_slice(&entry_count.to_le_bytes()); // entries on this disk
    output.extend_from_slice(&entry_count.to_le_bytes()); // total entries
    output.extend_from_slice(&cd_len.to_le_bytes());
    output.extend_from_slice(&cd_start.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // comment len

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    fn plan() -> StagingPlan {
        StagingPlan {
            transaction: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        }
    }
    #[test]
    fn generated_archive_has_exact_link_and_valid_crc_metadata() {
        let target = "/var/mobile/Library/Passes/Cards/AAoUHigyPEZQWmRueIKMlqCqtL4=.pkpass";
        let bytes = build_streaming_zip_archive_multi(
            target,
            &[("cardBackgroundCombined@3x.png", b"fixture")],
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            crate::safe_relative_path(entry.name().trim_end_matches('/')).unwrap();
            let mut contents = vec![];
            entry.read_to_end(&mut contents).unwrap();
            let mode = entry.unix_mode().unwrap();
            let extra = entry.extra_data().unwrap();
            assert!(extra.windows(6).any(|x| x[..4] == [0x53, 0x5a, 2, 0]
                && u16::from_le_bytes([x[4], x[5]]) == (mode & 0xffff) as u16));
            if entry.name() == "p0/p1/p2/link" {
                assert_eq!(mode & 0o170000, 0o120000);
                assert_eq!(contents, format!("../../..{}", target).into_bytes());
            }
        }
    }
    #[test]
    fn only_generated_link_transfer_is_accepted() {
        let p = plan();
        assert!(is_link_transfer(&p.asset_id(), &p.link()));
        for id in [
            "../../etc/passwd",
            "../../AirCard-Linux-Theme-x/p0/p1/p2/link",
        ] {
            assert!(!is_link_transfer(id, &p.link()));
        }
        assert!(!is_link_transfer(&p.asset_id(), "Books/other"));
        let mut p = plan();
        p.transaction = "../escape".into();
        assert!(p.validate().is_err());
    }
    #[test]
    fn request_preserves_normal_catalog_and_adds_only_owned_link() {
        let p = plan();
        let snapshot = crate::books::BooksSnapshot {
            entries: Default::default(),
        };
        let (bytes, ids) = p.request(&snapshot).unwrap();
        assert!(ids.is_empty());
        let value = crate::decode_binary(&bytes).unwrap();
        let row = &value.as_dictionary().unwrap()["Books"].as_array().unwrap()[0];
        assert_eq!(
            row.as_dictionary().unwrap()["Persistent ID"].as_string(),
            Some(p.asset_id().as_str())
        );
    }
}
