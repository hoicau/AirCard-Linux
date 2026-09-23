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
    let mut previous = capture(afc, cancelled)?;
    // ATC can finish before its known Books metadata writers become quiescent.
    // Retry only metadata changes; never mask a concurrent user-content mutation.
    for attempt in 0..10 {
        let current = capture(afc, cancelled)?;
        if previous == current {
            return Ok(current);
        }
        if previous
            .entries
            .keys()
            .chain(current.entries.keys())
            .any(|path| {
                previous.entries.get(path) != current.entries.get(path)
                    && !TRACKED_METADATA.contains(&path.as_str())
                    && !["Books", "Books/Sync", "Books/Sync/Database"].contains(&path.as_str())
            })
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                "books_content_changed_during_snapshot",
            ));
        }
        previous = current;
        if attempt < 9 {
            if cancelled() {
                return Err(Error::new(ErrorKind::Cancelled, "books_snapshot"));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    Err(Error::new(ErrorKind::Conflict, "books_snapshot_not_stable"))
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
    stage_payload(afc, snapshot, plan, Some(payload), cancelled, mutated)
}
/// Stage the fixed synthetic EPUB as an expanded directory, as used by Books.
/// Public reference: rk700/book2pad 6bf346e, addbooks() EPUB branch.
pub fn stage_epub(
    afc: &mut impl AfcAccess,
    snapshot: &BooksSnapshot,
    plan: &TestPlan,
    cancelled: &dyn Fn() -> bool,
    mutated: &mut bool,
) -> Result<()> {
    stage_payload(afc, snapshot, plan, None, cancelled, mutated)
}
fn stage_payload(
    afc: &mut impl AfcAccess,
    snapshot: &BooksSnapshot,
    plan: &TestPlan,
    payload: Option<&[u8]>,
    cancelled: &dyn Fn() -> bool,
    mutated: &mut bool,
) -> Result<()> {
    plan.validate()?;
    snapshot
        .validate()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "snapshot_validation"))?;
    if payload.is_some_and(|p| p.len() > MAX_FILE_BYTES) {
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
    *mutated = true;
    afc.mkdir(&plan.scratch_root)?;
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
    if let Some(payload) = payload {
        afc.write(&plan.source(), payload)?;
        if afc.read(&plan.source(), MAX_FILE_BYTES)? != payload {
            return Err(Error::new(ErrorKind::Native, "source_verify"));
        }
    } else {
        afc.mkdir(&plan.source())?;
        for dir in ["META-INF", "OEBPS"] {
            afc.mkdir(&format!("{}/{dir}", plan.source()))?;
        }
        for entry in aircard_core::books::synthetic_epub_entries() {
            if cancelled() {
                return Err(Error::new(ErrorKind::Cancelled, "stage_epub"));
            }
            afc.write(
                &format!("{}/{}", plan.source(), entry.relative_path),
                &entry.data,
            )?;
        }
        if !synthetic_epub_matches(afc, &plan.source())? {
            return Err(Error::new(ErrorKind::Native, "source_verify"));
        }
    }
    if cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "stage"));
    }
    atomic_replace(afc, "Books/Sync/Books.plist", &request, &plan.temporary())
}
/// Replace only sync metadata with a request produced by the typed customization plan.
pub fn stage_customization_request(
    afc: &mut impl AfcAccess,
    snapshot: &BooksSnapshot,
    plan: &TestPlan,
    request: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    plan.validate()?;
    if &capture_stable(afc, cancelled)? != snapshot {
        return Err(Error::new(
            ErrorKind::Conflict,
            "customization_snapshot_changed",
        ));
    }
    if let Some(SnapshotEntry::File(data)) = snapshot.entries.get("Books/Sync/Books.plist")
        && !aircard_core::books::sync_request_is_empty(data)
            .map_err(|_| Error::new(ErrorKind::Conflict, "customization_pending_unreadable"))?
    {
        return Err(Error::new(
            ErrorKind::Conflict,
            "customization_pending_sync",
        ));
    }
    aircard_core::decode_binary(request)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "customization_request"))?;
    for path in ["Books", "Books/Sync"] {
        if missing(afc, path)? {
            afc.mkdir(path)?;
        }
    }
    atomic_replace(afc, "Books/Sync/Books.plist", request, &plan.temporary())
}
pub fn synthetic_epub_matches(afc: &mut impl AfcAccess, root: &str) -> Result<bool> {
    if afc.stat(root)?.kind != FileKind::Directory {
        return Ok(false);
    }
    for entry in aircard_core::books::synthetic_epub_entries() {
        if afc.read(&format!("{root}/{}", entry.relative_path), MAX_FILE_BYTES)? != entry.data {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Remove only a transaction-owned source; bounded traversal never follows links.
fn remove_owned_source(afc: &mut impl AfcAccess, plan: &TestPlan) -> Result<()> {
    let mut pending = vec![plan.source()];
    let mut paths = Vec::new();
    while let Some(path) = pending.pop() {
        if paths.len() + pending.len() >= MAX_FILES {
            return Err(Error::new(ErrorKind::InvalidInput, "source_cleanup_limit"));
        }
        match afc.stat(&path) {
            Err(e) if e.kind == ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
            Ok(info) => match info.kind {
                FileKind::Directory => {
                    for leaf in afc.list(&path)? {
                        if leaf == "." || leaf == ".." {
                            continue;
                        }
                        aircard_core::safe_leaf(&leaf).map_err(|_| {
                            Error::new(ErrorKind::InvalidInput, "source_cleanup_path")
                        })?;
                        pending.push(format!("{path}/{leaf}"));
                    }
                }
                FileKind::File => {}
                _ => return Err(Error::new(ErrorKind::InvalidInput, "source_cleanup_type")),
            },
        }
        paths.push(path);
    }
    paths.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
    for path in paths {
        afc.remove(&path)?;
    }
    Ok(())
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
        || path.starts_with(&format!("{}/", plan.destination()))
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
    remove_owned_source(afc, plan)?;
    for path in [plan.scratch_root.clone()]
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
        changing_path: Option<String>,
        reads_to_change: u8,
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
            if self.changing_path.as_deref() == Some(path) && self.reads_to_change > 0 {
                self.reads_to_change -= 1;
                self.entries
                    .insert(path.into(), SnapshotEntry::File(vec![self.reads_to_change]));
            }
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
            let entries = self.entries.clone();
            for (path, entry) in entries {
                if path == source || path.starts_with(&format!("{source}/")) {
                    self.entries.remove(&path);
                    self.entries
                        .insert(format!("{target}{}", &path[source.len()..]), entry);
                }
            }
            Ok(())
        }
        fn remove(&mut self, path: &str) -> Result<()> {
            if self
                .entries
                .keys()
                .any(|p| p.starts_with(&format!("{path}/")))
            {
                return Err(Error::new(ErrorKind::Conflict, "directory_not_empty"));
            }
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
    fn expanded_epub_transfer_and_partial_source_are_fully_recoverable() {
        for transfer in [false, true] {
            let (mut mock, snapshot, plan) = setup();
            let mut touched = false;
            stage_epub(&mut mock, &snapshot, &plan, &|| false, &mut touched).unwrap();
            assert!(synthetic_epub_matches(&mut mock, &plan.source()).unwrap());
            if transfer {
                mock.rename(&plan.source(), &plan.destination()).unwrap();
                assert!(synthetic_epub_matches(&mut mock, &plan.destination()).unwrap());
            }
            restore(&mut mock, &snapshot, &plan).unwrap();
            assert_eq!(mock.entries, snapshot.entries);
        }
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
    #[test]
    fn settling_metadata_is_bounded_but_changing_book_content_is_not_retried() {
        for (leaf, settles) in [("Books.plist", true), ("reader.epub", false)] {
            let path = format!("Books/{leaf}");
            let mut mock = Mock {
                entries: BTreeMap::from([
                    ("Books".into(), SnapshotEntry::Directory),
                    (path.clone(), SnapshotEntry::File(vec![0])),
                ]),
                changing_path: Some(path),
                reads_to_change: 3,
                ..Default::default()
            };
            let result = capture_stable(&mut mock, &|| false);
            assert_eq!(result.is_ok(), settles);
            if !settles {
                assert_eq!(mock.reads_to_change, 1);
            }
        }
    }
}
