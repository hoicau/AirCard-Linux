//! Complete bounded Books snapshots through the device trait; no native handles here.
use crate::{AfcAccess, Error, ErrorKind, FileKind, Result};
use aircard_core::books::{
    BooksSnapshot, MAX_FILE_BYTES, MAX_FILES, MAX_SNAPSHOT_BYTES, SnapshotEntry,
};
use std::collections::BTreeMap;

pub fn capture(afc: &mut impl AfcAccess, cancelled: &dyn Fn() -> bool) -> Result<BooksSnapshot> {
    let mut entries = BTreeMap::new();
    let mut pending = vec!["Books".to_string()];
    let mut total = 0usize;
    while let Some(path) = pending.pop() {
        if cancelled() {
            return Err(Error::new(ErrorKind::Cancelled, "books_snapshot"));
        }
        if entries.len() >= MAX_FILES || path.split('/').count() > 16 {
            return Err(Error::new(ErrorKind::InvalidInput, "books_snapshot_limit"));
        }
        let info = match afc.stat(&path) {
            Ok(info) => info,
            Err(e) if path == "Books" && e.kind == ErrorKind::NotFound => break,
            Err(e) => return Err(e),
        };
        match info.kind {
            FileKind::Directory => {
                entries.insert(path.clone(), SnapshotEntry::Directory);
                for leaf in afc.list(&path)? {
                    if leaf == "." || leaf == ".." {
                        continue;
                    }
                    aircard_core::safe_leaf(&leaf)
                        .map_err(|_| Error::new(ErrorKind::InvalidInput, "books_snapshot_path"))?;
                    if pending.len() + entries.len() >= MAX_FILES {
                        return Err(Error::new(ErrorKind::InvalidInput, "books_snapshot_limit"));
                    }
                    pending.push(format!("{path}/{leaf}"));
                }
            }
            FileKind::File => {
                if info.size > MAX_FILE_BYTES as u64
                    || total.saturating_add(info.size as usize) > MAX_SNAPSHOT_BYTES
                {
                    return Err(Error::new(ErrorKind::InvalidInput, "books_snapshot_size"));
                }
                let data = afc.read(&path, MAX_FILE_BYTES)?;
                if data.len() as u64 != info.size {
                    return Err(Error::new(ErrorKind::Conflict, "books_snapshot_changed"));
                }
                total += data.len();
                entries.insert(path, SnapshotEntry::File(data));
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "books_snapshot_not_regular",
                ));
            }
        }
    }
    let snapshot = BooksSnapshot { entries };
    snapshot
        .validate()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "books_snapshot_invalid"))?;
    Ok(snapshot)
}
/// Two identical full reads are required before staging. This does not freeze iOS databases.
pub fn capture_stable(
    afc: &mut impl AfcAccess,
    cancelled: &dyn Fn() -> bool,
) -> Result<BooksSnapshot> {
    let first = capture(afc, cancelled)?;
    if first != capture(afc, cancelled)? {
        return Err(Error::new(ErrorKind::Conflict, "books_snapshot_not_stable"));
    }
    Ok(first)
}

pub const TRACKED_METADATA: &[&str] = &[
    "Books/Books.plist",
    "Books/Purchases/Purchases.plist",
    "Books/Sync/Books.plist",
    "Books/Sync/Upload.plist",
    "Books/Sync/Database/OutstandingAssets_4.sqlite",
    "Books/Sync/Database/OutstandingAssets_4.sqlite-shm",
    "Books/Sync/Database/OutstandingAssets_4.sqlite-wal",
];
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestPlan {
    pub asset_id: String,
    pub scratch_root: String,
    pub airlock_dirs_to_create: Vec<String>,
}
impl TestPlan {
    pub fn validate(&self) -> Result<()> {
        for leaf in [&self.asset_id, &self.scratch_root] {
            aircard_core::safe_leaf(leaf)
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "test_plan_path"))?;
        }
        if !self.asset_id.starts_with("AirCard-Linux-Test-")
            || !self.asset_id.ends_with(".epub")
            || !self.scratch_root.starts_with("AirCard-Linux-PoC-")
        {
            return Err(Error::new(ErrorKind::InvalidInput, "test_plan_scope"));
        }
        if !matches!(
            self.airlock_dirs_to_create
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice(),
            [] | ["Airlock/Book"] | ["Airlock", "Airlock/Book"]
        ) {
            return Err(Error::new(ErrorKind::InvalidInput, "airlock_scope"));
        }
        Ok(())
    }
    pub fn source(&self) -> String {
        format!("Airlock/Book/{}", self.asset_id)
    }
    pub fn destination(&self) -> String {
        format!("Books/{}", self.asset_id)
    }
    fn temporary(&self) -> String {
        format!("Books/Sync/{}.pending", self.asset_id)
    }
}
fn missing(afc: &mut impl AfcAccess, path: &str) -> Result<bool> {
    match afc.stat(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind == ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}
fn atomic_replace(
    afc: &mut impl AfcAccess,
    path: &str,
    data: &[u8],
    temporary: &str,
) -> Result<()> {
    if !missing(afc, temporary)? {
        return Err(Error::new(ErrorKind::Conflict, "books_temporary_collision"));
    }
    afc.write(temporary, data)?;
    if afc.read(temporary, MAX_FILE_BYTES)? != data {
        return Err(Error::new(ErrorKind::Native, "books_temporary_verify"));
    }
    afc.rename(temporary, path)?;
    if afc.read(path, MAX_FILE_BYTES)? != data {
        return Err(Error::new(ErrorKind::Native, "books_replace_verify"));
    }
    Ok(())
}
/// All changes are within normal AFC Books/scratch scope. Snapshot must already be saved durably.
pub fn stage(
    afc: &mut impl AfcAccess,
    snapshot: &BooksSnapshot,
    plan: &TestPlan,
    payload: &[u8],
    cancelled: &dyn Fn() -> bool,
    mutated: &mut bool,
) -> Result<()> {
    plan.validate()?;
    snapshot
        .validate()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "snapshot_validation"))?;
    if payload.len() > MAX_FILE_BYTES {
        return Err(Error::new(ErrorKind::InvalidInput, "payload_size"));
    }
    if &capture_stable(afc, cancelled)? != snapshot {
        return Err(Error::new(ErrorKind::Conflict, "snapshot_precondition"));
    }
    if let Some(SnapshotEntry::File(data)) = snapshot.entries.get("Books/Sync/Books.plist")
        && !aircard_core::books::sync_request_is_empty(data)
            .map_err(|_| Error::new(ErrorKind::Conflict, "existing_sync_request_unreadable"))?
    {
        return Err(Error::new(ErrorKind::Conflict, "existing_pending_sync"));
    }
    for path in [
        &plan.scratch_root,
        &plan.source(),
        &plan.destination(),
        &plan.temporary(),
    ] {
        // The temporary's parent may not exist yet; snapshot membership is enough there.
        if path == &plan.temporary() {
            if snapshot.entries.contains_key(path) {
                return Err(Error::new(ErrorKind::Conflict, "test_collision"));
            }
            continue;
        }
        if !missing(afc, path)? {
            return Err(Error::new(ErrorKind::Conflict, "test_collision"));
        }
    }
    if cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "stage"));
    }
    let request = aircard_core::books::preserving_books_plist(snapshot, &plan.asset_id)
        .map(|(bytes, _)| bytes)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "books_request"))?;
    afc.mkdir(&plan.scratch_root)?;
    *mutated = true;
    for path in &plan.airlock_dirs_to_create {
        if !missing(afc, path)? {
            return Err(Error::new(ErrorKind::Conflict, "airlock_dir_race"));
        }
        afc.mkdir(path)?;
    }
    for path in ["Books", "Books/Sync"] {
        if missing(afc, path)? {
            afc.mkdir(path)?;
        }
    }
    afc.write(&plan.source(), payload)?;
    if afc.read(&plan.source(), MAX_FILE_BYTES)? != payload {
        return Err(Error::new(ErrorKind::Native, "source_verify"));
    }
    if cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "stage"));
    }
    atomic_replace(afc, "Books/Sync/Books.plist", &request, &plan.temporary())
}
pub fn original_content_unchanged(
    afc: &mut impl AfcAccess,
    snapshot: &BooksSnapshot,
) -> Result<bool> {
    for (path, entry) in &snapshot.entries {
        if TRACKED_METADATA.contains(&path.as_str()) {
            continue;
        }
        match entry {
            SnapshotEntry::Directory => {
                if !matches!(afc.stat(path),Ok(info) if info.kind==FileKind::Directory) {
                    return Ok(false);
                }
            }
            SnapshotEntry::File(bytes) => {
                if !afc
                    .read(path, MAX_FILE_BYTES)
                    .is_ok_and(|current| current == *bytes)
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
fn owned_change(path: &str, plan: &TestPlan) -> bool {
    TRACKED_METADATA.contains(&path)
        || path == plan.destination()
        || path == plan.source()
        || path == plan.temporary()
        || ["Books", "Books/Sync", "Books/Sync/Database"].contains(&path)
}
/// Restore known metadata and this test's asset only. Unknown concurrent changes are preserved.
/// A conflict leaves the durable snapshot available for explicit recovery; it is never hidden.
pub fn restore(afc: &mut impl AfcAccess, snapshot: &BooksSnapshot, plan: &TestPlan) -> Result<()> {
    plan.validate()?;
    snapshot
        .validate()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "snapshot_validation"))?;
    let current = capture_stable(afc, &|| false)?;
    for path in current.entries.keys().chain(snapshot.entries.keys()) {
        if current.entries.get(path) != snapshot.entries.get(path)
            && current.entries.contains_key(path)
            && !owned_change(path, plan)
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                "unrelated_books_change_preserved",
            ));
        }
    }
    // Remove a partial transaction-owned temporary before resuming atomic replacements.
    if current.entries.contains_key(&plan.temporary()) {
        afc.remove(&plan.temporary())?;
    }
    let mut restore_dirs = Vec::new();
    if snapshot
        .entries
        .iter()
        .any(|(p, e)| matches!(e, SnapshotEntry::File(_)) && current.entries.get(p) != Some(e))
    {
        for path in ["Books", "Books/Sync"] {
            if missing(afc, path)? {
                afc.mkdir(path)?;
                if !snapshot.entries.contains_key(path) {
                    restore_dirs.push(path);
                }
            }
        }
    }
    for (path, entry) in &snapshot.entries {
        if current.entries.get(path) == Some(entry) {
            continue;
        }
        match entry {
            SnapshotEntry::Directory => {
                if missing(afc, path)? {
                    afc.mkdir(path)?;
                }
            }
            SnapshotEntry::File(data) => atomic_replace(afc, path, data, &plan.temporary())?,
        }
    }
    let mut added: Vec<_> = current
        .entries
        .keys()
        .filter(|p| !snapshot.entries.contains_key(*p))
        .cloned()
        .collect();
    added.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
    for path in added {
        if !missing(afc, &path)? {
            afc.remove(&path)?;
        }
    }
    for path in restore_dirs.into_iter().rev() {
        if !missing(afc, path)? {
            afc.remove(path)?;
        }
    }
    for path in [plan.source(), plan.scratch_root.clone()]
        .into_iter()
        .chain(plan.airlock_dirs_to_create.iter().rev().cloned())
    {
        if !missing(afc, &path)? {
            afc.remove(&path)?;
        }
    }
    if &capture_stable(afc, &|| false)? != snapshot {
        return Err(Error::new(ErrorKind::Conflict, "books_restore_verify"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mock {
        entries: BTreeMap<String, SnapshotEntry>,
        fail_write: bool,
        writes: usize,
    }
    impl AfcAccess for Mock {
        fn stat(&mut self, path: &str) -> Result<crate::FileInfo> {
            match self.entries.get(path) {
                Some(SnapshotEntry::Directory) => Ok(crate::FileInfo {
                    kind: FileKind::Directory,
                    size: 0,
                }),
                Some(SnapshotEntry::File(data)) => Ok(crate::FileInfo {
                    kind: FileKind::File,
                    size: data.len() as u64,
                }),
                None => Err(Error::new(ErrorKind::NotFound, "stat")),
            }
        }
        fn list(&mut self, path: &str) -> Result<Vec<String>> {
            let prefix = format!("{path}/");
            Ok(self
                .entries
                .keys()
                .filter_map(|p| p.strip_prefix(&prefix))
                .filter(|p| !p.contains('/'))
                .map(str::to_string)
                .collect())
        }
        fn read(&mut self, path: &str, _: usize) -> Result<Vec<u8>> {
            match self.entries.get(path) {
                Some(SnapshotEntry::File(data)) => Ok(data.clone()),
                _ => Err(Error::new(ErrorKind::NotFound, "read")),
            }
        }
        fn mkdir(&mut self, path: &str) -> Result<()> {
            self.entries.insert(path.into(), SnapshotEntry::Directory);
            Ok(())
        }
        fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
            if self.entries.contains_key(path) {
                return Err(Error::new(ErrorKind::Conflict, "write_exists"));
            }
            self.entries
                .insert(path.into(), SnapshotEntry::File(data.to_vec()));
            self.writes += 1;
            if self.fail_write {
                Err(Error::new(ErrorKind::Disconnected, "partial_write"))
            } else {
                Ok(())
            }
        }
        fn rename(&mut self, source: &str, target: &str) -> Result<()> {
            let entry = self.entries.remove(source).unwrap();
            self.entries.insert(target.into(), entry);
            Ok(())
        }
        fn remove(&mut self, path: &str) -> Result<()> {
            self.entries
                .remove(path)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "remove"))?;
            Ok(())
        }
        fn tls(&self) -> bool {
            false
        }
    }
    fn setup() -> (Mock, BooksSnapshot, TestPlan) {
        let snapshot = BooksSnapshot {
            entries: BTreeMap::from([
                ("Books".into(), SnapshotEntry::Directory),
                (
                    "Books/user-content.bin".into(),
                    SnapshotEntry::File(b"user book must survive".to_vec()),
                ),
                (
                    "Books/Books.plist".into(),
                    SnapshotEntry::File(br#"<?xml version="1.0"?><plist version="1.0"><dict><key>Books</key><array/></dict></plist>"#.to_vec()),
                ),
            ]),
        };
        let mock = Mock {
            entries: snapshot.entries.clone(),
            ..Mock::default()
        };
        (
            mock,
            snapshot,
            TestPlan {
                asset_id: "AirCard-Linux-Test-unit.epub".into(),
                scratch_root: "AirCard-Linux-PoC-unit".into(),
                airlock_dirs_to_create: vec!["Airlock".into(), "Airlock/Book".into()],
            },
        )
    }
    #[test]
    fn stage_roundtrip_and_full_restore_preserve_user_book() {
        let (mut mock, snapshot, plan) = setup();
        let mut touched = false;
        stage(
            &mut mock,
            &snapshot,
            &plan,
            b"synthetic",
            &|| false,
            &mut touched,
        )
        .unwrap();
        assert!(touched);
        mock.rename(&plan.source(), &plan.destination()).unwrap();
        mock.entries.insert(
            "Books/Books.plist".into(),
            SnapshotEntry::File(b"updated metadata".to_vec()),
        );
        restore(&mut mock, &snapshot, &plan).unwrap();
        assert_eq!(mock.entries, snapshot.entries);
    }
    #[test]
    fn partial_stage_failure_is_recoverable() {
        let (mut mock, snapshot, plan) = setup();
        let mut touched = false;
        mock.fail_write = true;
        assert!(
            stage(
                &mut mock,
                &snapshot,
                &plan,
                b"synthetic",
                &|| false,
                &mut touched
            )
            .is_err()
        );
        assert!(touched);
        mock.fail_write = false;
        restore(&mut mock, &snapshot, &plan).unwrap();
        assert_eq!(mock.entries, snapshot.entries);
    }
    #[test]
    fn user_changes_are_never_overwritten_during_restore() {
        let (mut mock, snapshot, plan) = setup();
        let mut touched = false;
        stage(
            &mut mock,
            &snapshot,
            &plan,
            b"synthetic",
            &|| false,
            &mut touched,
        )
        .unwrap();
        mock.entries.insert(
            "Books/new-user.epub".into(),
            SnapshotEntry::File(b"new user data".to_vec()),
        );
        let before = mock.entries.clone();
        assert_eq!(
            restore(&mut mock, &snapshot, &plan).unwrap_err().kind,
            ErrorKind::Conflict
        );
        assert_eq!(mock.entries, before);
    }
    #[test]
    fn cancelled_or_stale_snapshot_never_stages() {
        for cancel in [false, true] {
            let (mut mock, snapshot, plan) = setup();
            let mut touched = false;
            if !cancel {
                mock.entries.insert(
                    "Books/user-content.bin".into(),
                    SnapshotEntry::File(vec![5]),
                );
            }
            assert!(
                stage(
                    &mut mock,
                    &snapshot,
                    &plan,
                    b"synthetic",
                    &|| cancel,
                    &mut touched
                )
                .is_err()
            );
            assert!(!touched);
            assert_eq!(mock.writes, 0);
        }
    }
    #[test]
    fn idempotent_restore_does_not_rewrite_unchanged_files() {
        let (mut mock, snapshot, plan) = setup();
        restore(&mut mock, &snapshot, &plan).unwrap();
        restore(&mut mock, &snapshot, &plan).unwrap();
        assert_eq!(mock.writes, 0);
    }
}
