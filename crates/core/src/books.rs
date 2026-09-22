//! Bounded, platform-independent Books snapshot and synthetic-asset models.
use crate::{Error, decode_binary, encode_binary, safe_leaf, safe_relative_path};
use plist::{Dictionary, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_FILES: usize = 1024;
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BooksSnapshot {
    pub entries: BTreeMap<String, SnapshotEntry>,
}
impl BooksSnapshot {
    pub fn validate(&self) -> Result<(), Error> {
        if self.entries.len() > MAX_FILES {
            return Err(Error::Limit);
        }
        let mut total = 0usize;
        for (path, entry) in &self.entries {
            safe_relative_path(path)?;
            if path != "Books" && !path.starts_with("Books/") {
                return Err(Error::UnsafePath);
            }
            if path != "Books" {
                let parent = path.rsplit_once('/').ok_or(Error::UnsafePath)?.0;
                if self.entries.get(parent) != Some(&SnapshotEntry::Directory) {
                    return Err(Error::UnsafePath);
                }
            }
            match entry {
                SnapshotEntry::Directory => {}
                SnapshotEntry::File(data) => {
                    if data.len() > MAX_FILE_BYTES {
                        return Err(Error::Limit);
                    }
                    total = total.checked_add(data.len()).ok_or(Error::Limit)?;
                }
            }
        }
        if !self.entries.is_empty() && self.entries.get("Books") != Some(&SnapshotEntry::Directory)
        {
            return Err(Error::UnsafePath);
        }
        if total > MAX_SNAPSHOT_BYTES {
            return Err(Error::Limit);
        }
        Ok(())
    }
    pub fn byte_count(&self) -> usize {
        self.entries
            .values()
            .map(|entry| match entry {
                SnapshotEntry::Directory => 0,
                SnapshotEntry::File(data) => data.len(),
            })
            .sum()
    }
}
/// Produces a minimal normal Book request. Identifiers are leaves, never paths or links.
pub fn synthetic_books_plist(asset_id: &str) -> Result<Vec<u8>, Error> {
    safe_leaf(asset_id)?;
    if !asset_id.starts_with("AirCard-Linux-Test-") {
        return Err(Error::UnsafePath);
    }
    encode_binary(&Value::Dictionary(Dictionary::from_iter([(
        "Books",
        Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
            ("Persistent ID", Value::String(asset_id.into())),
            ("Item ID", Value::String("1".into())),
            ("DSID", Value::String("1".into())),
        ]))]),
    )])))
}
/// Stage only when the existing sync request is absent or has no pending user assets.
pub fn sync_request_is_empty(bytes: &[u8]) -> Result<bool, Error> {
    let value = decode_binary(bytes)?;
    Ok(value
        .as_dictionary()
        .and_then(|d| d.get("Books"))
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty))
}
/// Preserve all catalog rows verbatim; reject missing/unsafe identities and uncovered books.
pub fn preserving_books_plist(
    snapshot: &BooksSnapshot,
    asset_id: &str,
) -> Result<(Vec<u8>, std::collections::BTreeSet<String>), Error> {
    snapshot.validate()?;
    let mut rows = BTreeMap::new();
    let mut covered = std::collections::BTreeSet::new();
    for catalog in ["Books/Books.plist", "Books/Purchases/Purchases.plist"] {
        let Some(SnapshotEntry::File(bytes)) = snapshot.entries.get(catalog) else {
            continue;
        };
        let value = crate::decode_plist(bytes)?;
        let books = value
            .as_dictionary()
            .and_then(|d| d.get("Books"))
            .and_then(Value::as_array)
            .ok_or(Error::UnsafePath)?;
        for row in books {
            let d = row.as_dictionary().ok_or(Error::UnsafePath)?;
            let id = d
                .get("Persistent ID")
                .and_then(Value::as_string)
                .ok_or(Error::UnsafePath)?;
            safe_leaf(id)?;
            let path = d
                .get("Path")
                .and_then(Value::as_string)
                .ok_or(Error::UnsafePath)?;
            safe_relative_path(path)?;
            covered.insert(format!(
                "{}/{path}",
                catalog.rsplit_once('/').ok_or(Error::UnsafePath)?.0
            ));
            if id == asset_id || rows.insert(id.to_string(), row.clone()).is_some() {
                return Err(Error::UnsafePath);
            }
        }
    }
    for path in snapshot
        .entries
        .keys()
        .filter(|p| p.ends_with(".epub") || p.ends_with(".pdf"))
    {
        if !covered.contains(path) {
            return Err(Error::UnsafePath);
        }
    }
    if rows.len() > 127 {
        return Err(Error::Limit);
    }
    let preserved = rows.keys().cloned().collect();
    let request = decode_binary(&synthetic_books_plist(asset_id)?)?;
    let row = request
        .as_dictionary()
        .and_then(|d| d.get("Books"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or(Error::UnsafePath)?
        .clone();
    let mut books: Vec<_> = rows.into_values().collect();
    books.push(row);
    Ok((
        encode_binary(&Value::Dictionary(Dictionary::from_iter([(
            "Books",
            Value::Array(books),
        )])))?,
        preserved,
    ))
}

/// A valid, self-contained EPUB containing only synthetic text for end-to-end testing.
pub fn synthetic_epub() -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let entries = [
        ("mimetype", "application/epub+zip"),
        (
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        ),
        (
            "OEBPS/content.opf",
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:uuid:12b392b0-9e81-4d31-95c8-1cc37c8fb52e</dc:identifier><dc:title>AirCard Linux controlled sync test</dc:title><dc:language>en</dc:language><meta property="dcterms:modified">2026-09-22T00:00:00Z</meta></metadata><manifest><item id="page" href="content.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="page"/></spine></package>"#,
        ),
        (
            "OEBPS/nav.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><ol><li><a href="content.xhtml">Controlled test</a></li></ol></nav></body></html>"#,
        ),
        (
            "OEBPS/content.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>AirCard Linux controlled sync test</title></head><body><h1>AirCard Linux controlled sync test</h1><p>This is synthetic test data. The test restores the previous Books state and removes this asset.</p></body></html>"#,
        ),
    ];
    for (name, data) in entries {
        zip.start_file(
            name,
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o600),
        )?;
        zip.write_all(data.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserving_request_keeps_existing_rows_and_rejects_uncovered_books() {
        let row = Value::Dictionary(Dictionary::from_iter([
            ("Persistent ID", Value::String("existing-id".into())),
            ("Path", Value::String("existing.epub".into())),
            ("Name", Value::String("Synthetic existing title".into())),
        ]));
        let catalog = encode_binary(&Value::Dictionary(Dictionary::from_iter([(
            "Books",
            Value::Array(vec![row.clone()]),
        )])))
        .unwrap();
        let mut snapshot = BooksSnapshot {
            entries: BTreeMap::from([
                ("Books".into(), SnapshotEntry::Directory),
                ("Books/Purchases".into(), SnapshotEntry::Directory),
                (
                    "Books/Purchases/existing.epub".into(),
                    SnapshotEntry::Directory,
                ),
                (
                    "Books/Purchases/Purchases.plist".into(),
                    SnapshotEntry::File(catalog),
                ),
            ]),
        };
        let (bytes, ids) =
            preserving_books_plist(&snapshot, "AirCard-Linux-Test-unit.epub").unwrap();
        assert_eq!(
            ids,
            std::collections::BTreeSet::from(["existing-id".into()])
        );
        let request = decode_binary(&bytes).unwrap();
        let rows = request.as_dictionary().unwrap()["Books"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], row);
        snapshot
            .entries
            .insert("Books/orphan.epub".into(), SnapshotEntry::File(vec![1]));
        assert!(preserving_books_plist(&snapshot, "AirCard-Linux-Test-unit.epub").is_err());
    }
    #[test]
    fn snapshot_requires_closed_tree_and_books_scope() {
        for path in ["Other", "Books/../private", "Books/missing/child"] {
            let snapshot = BooksSnapshot {
                entries: BTreeMap::from([(path.into(), SnapshotEntry::File(vec![]))]),
            };
            assert!(snapshot.validate().is_err());
        }
        let snapshot = BooksSnapshot {
            entries: BTreeMap::from([
                ("Books".into(), SnapshotEntry::Directory),
                ("Books/a".into(), SnapshotEntry::File(vec![1])),
            ]),
        };
        assert!(snapshot.validate().is_ok());
    }
    #[test]
    fn request_rejects_escape_and_existing_requests_are_not_empty() {
        assert!(synthetic_books_plist("../../escape").is_err());
        let bytes = synthetic_books_plist("AirCard-Linux-Test-synthetic.epub").unwrap();
        assert!(!sync_request_is_empty(&bytes).unwrap());
    }
}
