//! Durable transaction journals and CLI policy; all device I/O uses AfcAccess.
use crate::{Bridge, Command, emit, local};
use aircard_core::books::{BooksSnapshot, MAX_FILE_BYTES, MAX_SNAPSHOT_BYTES};
use device::{AfcAccess, DeviceProvider, Error, ErrorKind, Result, books::TestPlan};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    schema_version: u32,
    device_fingerprint: String,
    snapshot: BooksSnapshot,
    plan: Option<TestPlan>,
    checksum: String,
}
impl Backup {
    fn checksum(&self) -> Result<String> {
        serde_json::to_vec(&(
            self.schema_version,
            &self.device_fingerprint,
            &self.snapshot,
            &self.plan,
        ))
        .map(|b| aircard_core::sha256(&b))
        .map_err(|_| invalid("backup_encode"))
    }
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.device_fingerprint.len() != 64
            || !self
                .device_fingerprint
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || self.checksum()? != self.checksum
        {
            return Err(invalid("backup_schema_or_checksum"));
        }
        self.snapshot
            .validate()
            .map_err(|_| invalid("backup_snapshot"))?;
        if let Some(plan) = &self.plan {
            plan.validate()?;
        }
        Ok(())
    }
    fn save(&mut self, path: &Path) -> Result<()> {
        self.checksum = self.checksum()?;
        self.validate()?;
        local::write_new(
            path,
            &serde_json::to_vec(self).map_err(|_| invalid("backup_encode"))?,
        )
    }
}
fn invalid(stage: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, stage)
}
fn load(path: &Path) -> Result<Backup> {
    let bytes = local::read(path, MAX_SNAPSHOT_BYTES * 4 + 4 * 1024 * 1024, true)?;
    let backup: Backup = serde_json::from_slice(&bytes).map_err(|_| invalid("backup_decode"))?;
    backup.validate()?;
    Ok(backup)
}
fn uuid() -> Result<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map_err(|_| Error::new(ErrorKind::Native, "random_id"))?;
    let id = id.trim().to_string();
    aircard_core::safe_leaf(&id).map_err(|_| invalid("random_id"))?;
    Ok(id)
}
fn plan(id: &str) -> TestPlan {
    TestPlan {
        asset_id: format!("AirCard-Linux-Test-{id}.epub"),
        scratch_root: format!("AirCard-Linux-PoC-{id}"),
        airlock_dirs_to_create: vec![],
    }
}
pub fn offline(command: &Command) -> Result<Option<u8>> {
    match command {
        Command::BooksRestore {
            input,
            apply: false,
        } => {
            let backup = load(input)?;
            emit(
                &json!({"event":"dry_run","applied":false,"scope":"complete bounded AFC Books tree","entries":backup.snapshot.entries.len(),"bytes":backup.snapshot.byte_count(),"recovery_journal":backup.plan.is_some(),"policy":"original device only; preserve unknown concurrent changes; verify full restoration"}),
            );
        }
        Command::BooksTest { apply: false, .. } => emit(
            &json!({"event":"dry_run","applied":false,"plan":["verify pairing and token profile","capture full bounded Books tree twice","save private durable journal","preserve catalog and stage one synthetic EPUB in Airlock/Book","require ReadyForSync and exact manifest","verify final asset bytes and original contents","restore full snapshot and delete journal only after verification"],"protected_paths":false}),
        ),
        _ => return Ok(None),
    }
    Ok(Some(0))
}
pub fn run(
    command: &Command,
    provider: &device::LinuxDeviceProvider,
    selected: &device::Device,
    timeout: u64,
    cancel: &AtomicBool,
) -> Result<u8> {
    let fingerprint = aircard_core::sha256(selected.udid.as_bytes());
    let cancelled = || cancel.load(Ordering::Relaxed);
    match command {
        Command::BooksSnapshot { output } => {
            let snapshot = device::books::capture_stable(&mut provider.afc(selected)?, &cancelled)?;
            let mut backup = Backup {
                schema_version: 1,
                device_fingerprint: fingerprint,
                snapshot,
                plan: None,
                checksum: String::new(),
            };
            backup.save(output)?;
            emit(
                &json!({"event":"books_snapshot_saved","entries":backup.snapshot.entries.len(),"bytes":backup.snapshot.byte_count(),"private":true}),
            );
            Ok(0)
        }
        Command::BooksRestore { input, apply: true } => {
            let backup = load(input)?;
            if fingerprint != backup.device_fingerprint {
                return Err(invalid("backup_device_mismatch"));
            }
            let mut afc = provider.afc(selected)?.with_books_write_scope();
            let plan = backup.plan.clone().unwrap_or(plan(&uuid()?));
            if cancelled() {
                return Err(Error::new(ErrorKind::Cancelled, "restore"));
            }
            device::books::restore(&mut afc, &backup.snapshot, &plan)?;
            // User-created snapshots remain under user control; transaction journals are temporary.
            if backup.plan.is_some() {
                local::remove(input)?;
            }
            emit(
                &json!({"event":"restore_complete","ok":true,"journal_removed":backup.plan.is_some()}),
            );
            Ok(0)
        }
        Command::BooksTest {
            journal,
            grappa_token,
            apply: true,
        } => {
            let token = local::token(grappa_token.as_deref())?
                .ok_or_else(|| invalid("books_test_requires_grappa_token"))?;
            let id = uuid()?;
            let mut afc = provider.afc(selected)?.with_books_write_scope();
            let snapshot = device::books::capture_stable(&mut afc, &cancelled)?;
            let mut plan = plan(&id);
            for path in ["Airlock", "Airlock/Book"] {
                match afc.stat(path) {
                    Ok(info) if info.kind == device::FileKind::Directory => {}
                    Err(e) if e.kind == ErrorKind::NotFound => {
                        plan.airlock_dirs_to_create.push(path.into())
                    }
                    _ => return Err(invalid("airlock_directory")),
                }
            }
            let payload =
                aircard_core::books::synthetic_epub().map_err(|_| invalid("synthetic_epub"))?;
            let (_, retained_ids) =
                aircard_core::books::preserving_books_plist(&snapshot, &plan.asset_id)
                    .map_err(|_| invalid("catalog_preservation"))?;
            let mut backup = Backup {
                schema_version: 1,
                device_fingerprint: fingerprint,
                snapshot,
                plan: Some(plan.clone()),
                checksum: String::new(),
            };
            backup.save(journal)?;
            emit(
                &json!({"event":"snapshot_saved","entries":backup.snapshot.entries.len(),"bytes":backup.snapshot.byte_count(),"synthetic_bytes":payload.len()}),
            );
            let mut touched = false;
            // Every fallible operation after staging stays inside this closure, so cleanup runs
            // on verification errors as well as protocol errors. Hard kills retain the journal.
            let operation = (|| -> Result<bool> {
                device::books::stage(
                    &mut afc,
                    &backup.snapshot,
                    &plan,
                    &payload,
                    &cancelled,
                    &mut touched,
                )?;
                let (_, service) =
                    device::open_verified_service(provider, selected, "com.apple.atc")?;
                let report = airtraffic::handshake::HandshakeClient {
                    transport: Bridge(service),
                }
                .synchronize_checked(
                    airtraffic::handshake::SyncOptions {
                        library_id: &id,
                        grappa: Some(&token),
                        timeout: Duration::from_secs(timeout),
                        cancelled: &cancelled,
                    },
                    &airtraffic::handshake::SyncAsset {
                        asset_id: plan.asset_id.clone(),
                        asset_path: plan.destination(),
                        retained_ids,
                    },
                    &mut || {
                        device::books::original_content_unchanged(&mut afc, &backup.snapshot)
                            .unwrap_or(false)
                    },
                    emit,
                );
                let protocol =
                    report.state == airtraffic::State::Finished && report.failure.is_none();
                emit(&report);
                let bytes_match = afc
                    .read(&plan.destination(), MAX_FILE_BYTES)
                    .is_ok_and(|data| data == payload);
                let preserved =
                    device::books::original_content_unchanged(&mut afc, &backup.snapshot)?;
                emit(
                    &json!({"event":"asset_verified","byte_match":bytes_match,"original_content_preserved":preserved}),
                );
                Ok(protocol && bytes_match && preserved)
            })();
            let restored = if touched {
                device::books::restore(&mut afc, &backup.snapshot, &plan)
            } else {
                Ok(())
            };
            emit(
                &json!({"event":"restore_complete","ok":restored.is_ok(),"error":restored.as_ref().err(),"journal_retained":restored.is_err()}),
            );
            if restored.is_ok() {
                local::remove(journal)?;
            }
            if let Err(e) = &operation {
                crate::error(e);
            }
            restored?;
            if cancelled() {
                return Ok(130);
            }
            Ok(if operation.unwrap_or(false) { 0 } else { 3 })
        }
        _ => unreachable!("offline commands handled before device selection"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_detects_modified_bytes_identity_or_plan() {
        let mut backup = Backup {
            schema_version: 1,
            device_fingerprint: "a".repeat(64),
            snapshot: BooksSnapshot {
                entries: Default::default(),
            },
            plan: None,
            checksum: String::new(),
        };
        backup.checksum = backup.checksum().unwrap();
        assert!(backup.validate().is_ok());
        backup.device_fingerprint = "b".repeat(64);
        assert!(backup.validate().is_err());
        backup.checksum = backup.checksum().unwrap();
        backup.plan = Some(plan("test"));
        assert!(backup.validate().is_err());
    }
}
