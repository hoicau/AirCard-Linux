//! Wallet-only application and restoration, sharing the tested native transaction engine.
use crate::{
    Bridge, Command,
    customization::{FileJournal, Journal, Session, finish, load, read_slot},
    emit, local,
};
use aircard_core::{
    assets::PreparedCard,
    books::MAX_FILE_BYTES,
    customization::{Plan, Step, Target},
    staging::StagingPlan,
};
use device::{AfcAccess, DeviceProvider, Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
fn invalid(op: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, op)
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Running,
    Committed,
    Cleaned,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default)]
    phase: Phase,
    schema: u32,
    device: String,
    card: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    target: Target,
    leaves: Vec<String>,
    originals: BTreeMap<usize, Vec<u8>>,
    applied: Vec<Vec<u8>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    schema: u32,
    device: String,
    parts: Vec<Part>,
    checksum: String,
}
impl Backup {
    fn checksum(&self) -> Result<String> {
        serde_json::to_vec(&(self.schema, &self.device, &self.parts))
            .map(|b| aircard_core::sha256(&b))
            .map_err(|_| invalid("card_backup_encode"))
    }
    fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.device.len() != 64
            || !self.device.bytes().all(|b| b.is_ascii_hexdigit())
            || self.parts.len() != 3
            || self.checksum != self.checksum()?
        {
            return Err(invalid("card_backup_integrity"));
        }
        let Target::WalletArtwork(hash) = &self.parts[0].target else {
            return Err(invalid("card_backup_target"));
        };
        for (i, part) in self.parts.iter().enumerate() {
            let expected = target(hash, i);
            if part.target != expected
                || part.leaves != leaves(i)
                || part.applied.len() != part.leaves.len()
                || part.originals.keys().any(|&i| i >= part.leaves.len())
                || part
                    .originals
                    .values()
                    .chain(&part.applied)
                    .any(|b| b.len() > MAX_FILE_BYTES)
            {
                return Err(invalid("card_backup_scope"));
            }
            expected
                .validate()
                .map_err(|_| invalid("card_backup_card"))?;
        }
        if self.parts[0].originals.is_empty() {
            return Err(invalid("card_backup_empty_artwork"));
        }
        if self
            .parts
            .iter()
            .flat_map(|p| p.originals.values().chain(&p.applied))
            .map(Vec::len)
            .sum::<usize>()
            > 128 * 1024 * 1024
        {
            return Err(invalid("card_backup_size"));
        }
        Ok(())
    }
    fn from_journals(device: &str, journals: &[Journal]) -> Result<Self> {
        let mut b = Self {
            schema: 1,
            device: device.into(),
            parts: journals
                .iter()
                .map(|j| Part {
                    target: j.plan.target.clone(),
                    leaves: j.plan.leaves.clone(),
                    originals: j.originals.clone(),
                    applied: j.payloads.clone(),
                })
                .collect(),
            checksum: String::new(),
        };
        b.checksum = b.checksum()?;
        b.validate()?;
        Ok(b)
    }
}
fn read_backup(path: &Path) -> Result<Backup> {
    let bytes = local::read_context(path, 520 * 1024 * 1024, true, "card_backup_input")?;
    let b: Backup = serde_json::from_slice(&bytes).map_err(|_| invalid("card_backup_decode"))?;
    b.validate()?;
    Ok(b)
}
fn check_preview(card: &PreparedCard, expected: Option<&str>) -> Result<()> {
    if expected.is_some_and(|hash| hash != aircard_core::sha256(&card.png)) {
        return Err(Error::new(
            ErrorKind::Conflict,
            "card_image_changed_since_preview",
        ));
    }
    Ok(())
}
fn target(hash: &str, index: usize) -> Target {
    match index {
        0 => Target::WalletArtwork(hash.into()),
        1 => Target::WalletCache {
            hash: hash.into(),
            extension: "cache".into(),
        },
        _ => Target::WalletCache {
            hash: hash.into(),
            extension: "pkcache".into(),
        },
    }
}
fn leaves(index: usize) -> Vec<String> {
    let values = if index == 0 {
        vec![
            "cardBackgroundCombined@3x.png",
            "cardBackgroundCombined@2x.png",
            "cardBackgroundCombined.pdf",
        ]
    } else {
        vec!["FrontFace", "PlaceHolder", "Preview"]
    };
    values.into_iter().map(str::to_string).collect()
}
pub fn offline(command: &Command) -> Result<Option<u8>> {
    match command {
        Command::CardTest {
            card_hash,
            apply: false,
            ..
        }
        | Command::CardApply {
            card_hash,
            apply: false,
            ..
        } => {
            target(card_hash, 0)
                .validate()
                .map_err(|_| invalid("card_hash"))?;
            if let Command::CardApply {
                input,
                expected_artwork_sha256,
                ..
            } = command
            {
                let card = PreparedCard::from_bytes(&local::read(
                    input,
                    aircard_core::assets::MAX_IMAGE_BYTES,
                    false,
                )?)
                .map_err(|_| invalid("card_image"))?;
                check_preview(&card, expected_artwork_sha256.as_deref())?;
            }
            emit(
                &json!({"event":"dry_run","applied":false,"scope":"selected Wallet card artwork and its generated display caches","plan":["private device-bound transaction directory","back up existing artwork and caches","preserve Books catalog","install and read back card image","invalidate display caches","restore Books and remove staging"],"test_restores_automatically":matches!(command,Command::CardTest{..})}),
            );
        }
        Command::CardRestore {
            input,
            apply: false,
            ..
        } => {
            let backup = read_backup(input)?;
            emit(
                &json!({"event":"dry_run","applied":false,"scope":"restore selected card artwork and cache preimages on its original device","parts":backup.parts.len()}),
            );
        }
        Command::CardRecover {
            journal,
            apply: false,
            ..
        } => {
            let (_, parts) = read_journal(journal)?;
            emit(
                &json!({"event":"dry_run","applied":false,"scope":"recover interrupted card operation on its original device","parts":parts.len()}),
            );
        }
        _ => return Ok(None),
    }
    Ok(Some(0))
}
fn journal_temporary(name: &str) -> bool {
    name.strip_prefix(".aircard-journal-")
        .and_then(|n| n.strip_suffix(".tmp"))
        .is_some_and(|id| aircard_core::staging::validate_transaction(id).is_ok())
}
fn read_journal(dir: &Path) -> Result<(Manifest, Vec<Journal>)> {
    let info = std::fs::symlink_metadata(dir).map_err(|_| invalid("card_journal_directory"))?;
    if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
        return Err(invalid("card_journal_permissions"));
    }
    let m: Manifest = serde_json::from_slice(&local::read(&dir.join("manifest.json"), 4096, true)?)
        .map_err(|_| invalid("card_journal_manifest"))?;
    if m.schema != 1 {
        return Err(invalid("card_journal_schema"));
    }
    target(&m.card, 0)
        .validate()
        .map_err(|_| invalid("card_hash"))?;
    let entries = std::fs::read_dir(dir)
        .map_err(|_| invalid("card_journal_list"))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| invalid("card_journal_list"))?;
    if entries.iter().any(|e| {
        !e.file_name().to_str().is_some_and(|name| {
            matches!(name, "manifest.json" | "0.json" | "1.json" | "2.json")
                || journal_temporary(name)
        })
    }) {
        return Err(invalid("card_journal_unexpected_file"));
    }
    let mut parts = vec![];
    for i in 0..3 {
        let path = dir.join(format!("{i}.json"));
        if path.exists() {
            let (j, _) = load(&path)?;
            if j.device != m.device
                || j.plan.target != target(&m.card, i)
                || j.plan.leaves != leaves(i)
            {
                return Err(invalid("card_journal_part"));
            }
            parts.push(j);
        } else {
            if m.phase != Phase::Cleaned
                && ((i + 1)..3).any(|next| dir.join(format!("{next}.json")).exists())
            {
                return Err(invalid("card_journal_part_gap"));
            }
            break;
        }
    }
    if m.phase == Phase::Committed && parts.len() != 3 {
        return Err(invalid("card_committed_parts_missing"));
    }
    Ok((m, parts))
}
fn set_phase(dir: &Path, phase: Phase) -> Result<()> {
    let path = dir.join("manifest.json");
    let old = local::read(&path, 4096, true)?;
    let mut m: Manifest =
        serde_json::from_slice(&old).map_err(|_| invalid("card_journal_manifest"))?;
    m.phase = phase;
    local::replace_journal(
        &path,
        &old,
        &serde_json::to_vec(&m).map_err(|_| invalid("card_journal_manifest"))?,
    )
}
fn clear_journal(dir: &Path, _count: usize) -> Result<()> {
    set_phase(dir, Phase::Cleaned)?;
    for i in 0..3 {
        let path = dir.join(format!("{i}.json"));
        if path
            .try_exists()
            .map_err(|_| invalid("card_journal_stat"))?
        {
            local::remove(&path)?;
        }
    }
    for entry in std::fs::read_dir(dir).map_err(|_| invalid("card_journal_list"))? {
        let entry = entry.map_err(|_| invalid("card_journal_list"))?;
        if entry.file_name().to_str().is_some_and(journal_temporary) {
            // Only the exact private atomic-write temporary format is owned by this transaction.
            let info = entry
                .path()
                .symlink_metadata()
                .map_err(|_| invalid("card_journal_temp_stat"))?;
            if !info.is_file() || info.permissions().mode() & 0o077 != 0 {
                return Err(invalid("card_journal_temp_type"));
            }
            local::remove(&entry.path())?;
        }
    }
    local::remove(&dir.join("manifest.json"))?;
    local::remove_empty_directory(dir)
}
fn restore_part(
    session: &Session<'_>,
    afc: &mut device::LinuxAfc,
    j: &mut Journal,
    file: &mut FileJournal<'_>,
) -> Result<()> {
    // Display caches may have been regenerated while the owner inspected the card. Clear
    // every known cache leaf before restoring preimages, including originally absent leaves.
    if matches!(j.plan.target, Target::WalletCache { .. }) && j.export_started && !j.restored {
        if j.rollback_round >= 8 {
            return Err(invalid("card_cache_recovery_limit"));
        }
        let round = j.rollback_round;
        j.rollback_round += 1;
        file.save(j)?;
        afc.mkdir(&format!("{}/rollback-{round}", j.plan.work()))?;
        let indices: Vec<_> = (0..j.plan.leaves.len()).collect();
        session.sync(afc, j, Step::Rollback(round), &indices, &|| false)?;
    }
    session.restore(afc, j, file)
}
fn restore_all(session: &Session<'_>, dir: &Path, journals: &mut [Journal]) -> Result<()> {
    emit(&json!({"event":"stage","stage":"restore_originals"}));
    let mut first_error = None;
    // Restore artwork first; cache restoration then reflects the restored artwork.
    for (i, j) in journals.iter_mut().enumerate() {
        let result = (|| -> Result<()> {
            let path = dir.join(format!("{i}.json"));
            let (_, bytes) = load(&path)?;
            let mut file = FileJournal { path: &path, bytes };
            let mut afc = session
                .provider
                .afc(session.selected)?
                .with_customization_scope(&j.plan)?;
            restore_part(session, &mut afc, j, &mut file)?;
            finish(&mut afc, j)
        })();
        if let Err(e) = result {
            emit(&json!({"event":"card_recovery_part_failed","part":i,"error":e}));
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }
    if let Some(e) = first_error {
        Err(e)
    } else {
        clear_journal(dir, journals.len())
    }
}
fn stage(
    session: &Session<'_>,
    afc: &mut device::LinuxAfc,
    j: &Journal,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    emit(&json!({"event":"stage","stage":"prepare_transfer"}));
    let archive = j
        .plan
        .archive(&j.payloads)
        .map_err(|_| invalid("card_archive"))?;
    let (_, service) = device::open_verified_service(
        session.provider,
        session.selected,
        "com.apple.streaming_zip_conduit",
    )?;
    let report = airtraffic::streaming_zip::stage(
        Bridge(service),
        &j.plan.source(),
        &archive,
        Duration::from_secs(session.timeout),
        cancelled,
    );
    let ok = report.complete;
    emit(&report);
    if !ok {
        return Err(invalid("card_staging"));
    }
    afc.customization_staged(&j.plan)?;
    afc.mkdir(&j.plan.work())?;
    for group in ["original", "verified", "restore"] {
        afc.mkdir(&format!("{}/{group}", j.plan.work()))?;
    }
    session.sync(afc, j, Step::Link, &[], cancelled)
}
fn install(
    session: &Session<'_>,
    afc: &mut device::LinuxAfc,
    j: &mut Journal,
    file: &mut FileJournal<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let indices: Vec<_> = (0..j.plan.leaves.len()).collect();
    j.export_started = true;
    file.save(j)?;
    emit(
        &json!({"event":"stage","stage":if matches!(j.plan.target, Target::WalletCache { .. }) { "refresh_caches" } else { "read_original_artwork" }}),
    );
    session.sync(afc, j, Step::ExportOriginals, &indices, cancelled)?;
    for i in indices {
        if let Some(bytes) = read_slot(afc, &j.plan.slot("original", i))? {
            j.originals.insert(i, bytes);
        }
    }
    file.save(j)?;
    emit(
        &json!({"event":"card_part_backed_up","cache":matches!(j.plan.target,Target::WalletCache{..}),"files":j.originals.len(),"bytes":j.originals.values().map(Vec::len).sum::<usize>()}),
    );
    if matches!(j.plan.target, Target::WalletCache { .. }) {
        // Moving generated cache entries out invalidates them. Wallet regenerates them from artwork.
        emit(&json!({"event":"wallet_cache_invalidated","files":j.originals.len()}));
        return Ok(());
    }
    if j.originals.is_empty() {
        return Err(invalid("card_artwork_not_found"));
    }
    let active: Vec<_> = j.originals.keys().copied().collect();
    j.apply_started = true;
    file.save(j)?;
    emit(&json!({"event":"stage","stage":"write_artwork"}));
    session.sync(afc, j, Step::Install, &active, cancelled)?;
    for &i in &active {
        if read_slot(afc, &format!("{}/payload_{i}", j.plan.source()))?.is_some() {
            return Err(invalid("card_install_move_failed"));
        }
    }
    emit(&json!({"event":"stage","stage":"verify_artwork"}));
    session.sync(afc, j, Step::ExportInstalled, &active, cancelled)?;
    for &i in &active {
        if read_slot(afc, &j.plan.slot("verified", i))?.as_ref() != Some(&j.payloads[i]) {
            return Err(invalid("card_readback_mismatch"));
        }
    }
    session.sync(afc, j, Step::ReturnInstalled, &active, cancelled)?;
    for &i in &active {
        if read_slot(afc, &j.plan.slot("verified", i))?.is_some() {
            return Err(invalid("card_return_failed"));
        }
    }
    emit(&json!({"event":"card_artwork_verified","files":active.len(),"readback_verified":true}));
    Ok(())
}
pub fn run(
    command: &Command,
    provider: &device::LinuxDeviceProvider,
    selected: &device::Device,
    timeout: u64,
    cancel: &AtomicBool,
) -> Result<u8> {
    let _lease = local::DeviceLease::acquire()?;
    let fingerprint = aircard_core::sha256(selected.udid.as_bytes());
    let cancelled = || cancel.load(Ordering::Relaxed);
    let (journal, token_path) = match command {
        Command::CardTest {
            journal,
            grappa_token,
            ..
        }
        | Command::CardApply {
            journal,
            grappa_token,
            ..
        }
        | Command::CardRestore {
            journal,
            grappa_token,
            ..
        }
        | Command::CardRecover {
            journal,
            grappa_token,
            ..
        } => (journal, grappa_token),
        _ => unreachable!(),
    };
    let token = local::token(Some(token_path))?.ok_or_else(|| invalid("card_token"))?;
    let session = Session {
        provider,
        selected,
        token: &token,
        timeout,
    };
    if matches!(command, Command::CardRecover { .. }) {
        emit(&json!({"event":"stage","stage":"restore_originals"}));
        let (m, mut journals) = read_journal(journal)?;
        if m.device != fingerprint {
            return Err(invalid("card_device_mismatch"));
        }
        if m.phase == Phase::Running {
            restore_all(&session, journal, &mut journals)?;
        } else {
            if m.phase == Phase::Committed {
                for j in &journals {
                    let mut afc = provider.afc(selected)?.with_customization_scope(&j.plan)?;
                    finish(&mut afc, j)?;
                }
            }
            clear_journal(journal, journals.len())?;
        }
        emit(&json!({"event":"card_recovery_complete","ok":true}));
        return Ok(0);
    }
    let restore_backup = if let Command::CardRestore { input, .. } = command {
        Some(read_backup(input)?)
    } else {
        None
    };
    if restore_backup
        .as_ref()
        .is_some_and(|b| b.device != fingerprint)
    {
        return Err(invalid("card_backup_device_mismatch"));
    }
    let (hash, card, hold, backup_path) = match command {
        Command::CardTest {
            card_hash,
            hold_seconds,
            ..
        } => (
            card_hash.clone(),
            Some(aircard_core::assets::test_card().map_err(|_| invalid("card_test_image"))?),
            Some(*hold_seconds),
            None,
        ),
        Command::CardApply {
            card_hash,
            input,
            backup,
            ..
        } => (
            card_hash.clone(),
            Some(
                PreparedCard::from_bytes(&local::read(
                    input,
                    aircard_core::assets::MAX_IMAGE_BYTES,
                    false,
                )?)
                .map_err(|_| invalid("card_image"))?,
            ),
            None,
            Some(backup),
        ),
        Command::CardRestore { .. } => {
            let Target::WalletArtwork(hash) = &restore_backup.as_ref().unwrap().parts[0].target
            else {
                unreachable!()
            };
            (hash.clone(), None, None, None)
        }
        _ => unreachable!(),
    };
    if let (
        Some(card),
        Command::CardApply {
            expected_artwork_sha256,
            ..
        },
    ) = (&card, command)
    {
        check_preview(card, expected_artwork_sha256.as_deref())?;
    }
    target(&hash, 0)
        .validate()
        .map_err(|_| invalid("card_hash"))?;
    if let Some(path) = backup_path
        && path.exists()
    {
        return Err(Error::new(
            ErrorKind::Conflict,
            "card_backup_already_exists",
        ));
    }
    local::create_private_directory(journal)?;
    let manifest = Manifest {
        phase: Phase::Running,
        schema: 1,
        device: fingerprint.clone(),
        card: hash.clone(),
    };
    if let Err(e) = local::write_new(
        &journal.join("manifest.json"),
        &serde_json::to_vec(&manifest).map_err(|_| invalid("card_manifest_encode"))?,
    ) {
        let _ = std::fs::remove_dir(journal);
        return Err(e);
    }
    let mut journals = Vec::new();
    let operation = (|| -> Result<()> {
        for part in 0..3 {
            let transaction = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
                .map_err(|_| invalid("card_transaction"))?
                .trim()
                .to_string();
            let plan = Plan {
                transaction,
                target: target(&hash, part),
                leaves: leaves(part),
            };
            let mut afc = provider.afc(selected)?.with_customization_scope(&plan)?;
            afc.staging_preflight(&StagingPlan {
                transaction: plan.transaction.clone(),
            })?;
            emit(&json!({"event":"stage","stage":"preserve_catalog"}));
            let books = device::books::capture_stable(&mut afc, &cancelled)?;
            let payloads = if let Some(b) = &restore_backup {
                b.parts[part].applied.clone()
            } else if part == 0 {
                card.as_ref()
                    .unwrap()
                    .resources()
                    .into_iter()
                    .map(|r| r.data)
                    .collect()
            } else {
                vec![vec![]; 3]
            };
            let mut j = Journal {
                schema: 1,
                device: fingerprint.clone(),
                plan,
                books,
                originals: BTreeMap::new(),
                payloads,
                export_started: false,
                apply_started: false,
                restored: false,
                rollback_round: 0,
                checksum: String::new(),
            };
            let path = journal.join(format!("{part}.json"));
            let bytes = j.bytes()?;
            local::write_new(&path, &bytes)?;
            journals.push(j);
            let j = journals.last_mut().unwrap();
            let mut file = FileJournal { path: &path, bytes };
            stage(&session, &mut afc, j, &cancelled)?;
            if let Some(backup) = &restore_backup {
                emit(&json!({"event":"stage","stage":"restore_originals"}));
                j.originals = backup.parts[part].originals.clone();
                j.export_started = true;
                j.apply_started = true;
                file.save(j)?;
                restore_part(&session, &mut afc, j, &mut file)?;
            } else {
                install(&session, &mut afc, j, &mut file, &cancelled)?;
            }
            emit(&json!({"event":"stage","stage":"restore_catalog"}));
            device::books::restore(&mut afc, &j.books, &j.books_plan())?;
        }
        if let Some(path) = backup_path {
            emit(&json!({"event":"stage","stage":"save_backup"}));
            let backup = Backup::from_journals(&fingerprint, &journals)?;
            local::write_new(
                path,
                &serde_json::to_vec(&backup).map_err(|_| invalid("card_backup_encode"))?,
            )?;
            emit(&json!({"event":"card_backup_saved","parts":backup.parts.len(),"private":true}));
        }
        if let Some(seconds) = hold {
            emit(
                &json!({"event":"card_inspection_ready","seconds":seconds,"artwork_readback_verified":true,"hint":"Close and reopen Wallet and inspect the selected card's blue/teal diagonal artwork. Close Wallet afterward; cancellation begins restoration."}),
            );
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(seconds) && !cancelled() {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ok(())
    })();
    if hold.is_some() || operation.is_err() {
        let recovered = restore_all(&session, journal, &mut journals);
        emit(
            &json!({"event":"card_test_complete","ok":operation.is_ok() && recovered.is_ok(),"operation_error":operation.as_ref().err(),"recovery_error":recovered.as_ref().err(),"journal_retained":recovered.is_err()}),
        );
        recovered?;
        operation?;
        return Ok(if cancelled() { 130 } else { 0 });
    }
    set_phase(journal, Phase::Committed)?;
    emit(&json!({"event":"stage","stage":"cleanup"}));
    for j in &journals {
        let mut afc = provider.afc(selected)?.with_customization_scope(&j.plan)?;
        finish(&mut afc, j)?;
    }
    clear_journal(journal, journals.len())?;
    emit(
        &json!({"event":"card_operation_complete","ok":true,"restored":restore_backup.is_some(),"backup_saved":backup_path.is_some()}),
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn backup() -> Backup {
        let mut b = Backup {
            schema: 1,
            device: "a".repeat(64),
            checksum: String::new(),
            parts: (0..3)
                .map(|i| Part {
                    target: target("AAoUHigyPEZQWmRueIKMlqCqtL4=", i),
                    leaves: leaves(i),
                    originals: if i == 0 {
                        BTreeMap::from([(0, vec![1, 2, 3])])
                    } else {
                        BTreeMap::new()
                    },
                    applied: vec![vec![4]; 3],
                })
                .collect(),
        };
        b.checksum = b.checksum().unwrap();
        b
    }
    #[test]
    fn backup_integrity_card_binding_and_scope_are_required() {
        let mut b = backup();
        assert!(b.validate().is_ok());
        b.parts[0].originals.get_mut(&0).unwrap().push(9);
        assert!(b.validate().is_err());
        b.checksum = b.checksum().unwrap();
        assert!(b.validate().is_ok());
        b.parts[1].target = target("../other", 1);
        b.checksum = b.checksum().unwrap();
        assert!(b.validate().is_err());
        let mut b = backup();
        b.parts[0].leaves[0] = "pass.json".into();
        b.checksum = b.checksum().unwrap();
        assert!(b.validate().is_err());
        let mut b = backup();
        b.parts[0].originals.clear();
        b.checksum = b.checksum().unwrap();
        assert!(b.validate().is_err());
    }
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let id = std::fs::read_to_string("/proc/sys/kernel/random/uuid").unwrap();
            let path = std::env::temp_dir().join(format!("aircard-wallet-fixture-{}", id.trim()));
            local::create_private_directory(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn journal_interrupted_atomic_write_and_cleanup_are_recoverable() {
        let dir = Directory::new();
        let m = Manifest {
            phase: Phase::Running,
            schema: 1,
            device: "a".repeat(64),
            card: "AAoUHigyPEZQWmRueIKMlqCqtL4=".into(),
        };
        local::write_new(
            &dir.0.join("manifest.json"),
            &serde_json::to_vec(&m).unwrap(),
        )
        .unwrap();
        let tmp = dir
            .0
            .join(".aircard-journal-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.tmp");
        local::write_new(&tmp, b"partial atomic write").unwrap();
        assert!(read_journal(&dir.0).is_ok());
        local::write_new(&dir.0.join("unrelated"), b"do not remove").unwrap();
        assert!(read_journal(&dir.0).is_err());
        std::fs::remove_file(dir.0.join("unrelated")).unwrap();
        set_phase(&dir.0, Phase::Committed).unwrap();
        assert!(read_journal(&dir.0).is_err()); // committed requires all three durable parts
        set_phase(&dir.0, Phase::Cleaned).unwrap();
        assert!(read_journal(&dir.0).is_ok());
        clear_journal(&dir.0, 3).unwrap();
        assert!(!dir.0.exists());
    }
    #[test]
    fn changed_image_is_rejected_after_preview() {
        let card = aircard_core::assets::test_card().unwrap();
        assert!(check_preview(&card, Some(&aircard_core::sha256(&card.png))).is_ok());
        assert_eq!(
            check_preview(&card, Some("old-preview")).unwrap_err().kind,
            ErrorKind::Conflict
        );
    }
}
