//! Applied Wallet transactions with durable originals, read-back verification and bounded recovery.
use crate::{Bridge, emit, local};
use aircard_core::{
    books::{BooksSnapshot, MAX_FILE_BYTES, MAX_SNAPSHOT_BYTES},
    customization::{Plan, Step, Target},
    staging::StagingPlan,
};
use device::{AfcAccess, Error, ErrorKind, Result, books::TestPlan};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, time::Duration};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    pub(crate) schema: u32,
    pub(crate) device: String,
    pub(crate) plan: Plan,
    pub(crate) books: BooksSnapshot,
    pub(crate) originals: BTreeMap<usize, Vec<u8>>,
    pub(crate) payloads: Vec<Vec<u8>>,
    pub(crate) export_started: bool,
    pub(crate) apply_started: bool,
    pub(crate) restored: bool,
    pub(crate) rollback_round: u8,
    pub(crate) checksum: String,
}
impl Journal {
    fn checksum(&self) -> Result<String> {
        serde_json::to_vec(&(
            self.schema,
            &self.device,
            &self.plan,
            &self.books,
            &self.originals,
            &self.payloads,
            self.export_started,
            self.apply_started,
            self.restored,
            self.rollback_round,
        ))
        .map(|bytes| aircard_core::sha256(&bytes))
        .map_err(|_| invalid("customization_journal_encode"))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.device.len() != 64
            || !self.device.bytes().all(|b| b.is_ascii_hexdigit())
            || self.checksum != self.checksum()?
            || self.rollback_round > 8
        {
            return Err(invalid("customization_journal_integrity"));
        }
        self.plan
            .validate()
            .map_err(|_| invalid("customization_plan"))?;
        self.books
            .validate()
            .map_err(|_| invalid("customization_books"))?;
        if self.payloads.len() != self.plan.leaves.len()
            || self.originals.keys().any(|&i| i >= self.payloads.len())
            || self
                .originals
                .values()
                .chain(&self.payloads)
                .any(|p| p.len() > MAX_FILE_BYTES)
            || self.originals.values().map(Vec::len).sum::<usize>() > MAX_SNAPSHOT_BYTES
            || self.payloads.iter().map(Vec::len).sum::<usize>() > MAX_SNAPSHOT_BYTES
        {
            return Err(invalid("customization_journal_limits"));
        }
        Ok(())
    }
    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>> {
        self.checksum = self.checksum()?;
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| invalid("customization_journal_encode"))
    }
    pub(crate) fn books_plan(&self) -> TestPlan {
        TestPlan {
            asset_id: format!("AirCard-Linux-Test-{}.epub", self.plan.transaction),
            scratch_root: format!("AirCard-Linux-PoC-{}", self.plan.transaction),
            airlock_dirs_to_create: vec![],
        }
    }
    pub(crate) fn cleanup_plan(&self) -> StagingPlan {
        StagingPlan {
            transaction: self.plan.transaction.clone(),
        }
    }
}
pub(crate) struct FileJournal<'a> {
    pub(crate) path: &'a Path,
    pub(crate) bytes: Vec<u8>,
}
impl FileJournal<'_> {
    pub(crate) fn save(&mut self, j: &mut Journal) -> Result<()> {
        let bytes = j.bytes()?;
        local::replace_journal(self.path, &self.bytes, &bytes)?;
        self.bytes = bytes;
        Ok(())
    }
}
fn invalid(op: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, op)
}
pub(crate) fn load(path: &Path) -> Result<(Journal, Vec<u8>)> {
    let bytes = local::read(path, MAX_SNAPSHOT_BYTES * 12 + 4 * 1024 * 1024, true)?;
    let j: Journal =
        serde_json::from_slice(&bytes).map_err(|_| invalid("customization_journal_decode"))?;
    j.validate()?;
    Ok((j, bytes))
}
pub(crate) struct Session<'a> {
    pub(crate) provider: &'a device::LinuxDeviceProvider,
    pub(crate) selected: &'a device::Device,
    pub(crate) token: &'a [u8],
    pub(crate) timeout: u64,
}
impl Session<'_> {
    pub(crate) fn sync(
        &self,
        afc: &mut device::LinuxAfc,
        j: &Journal,
        step: Step,
        indices: &[usize],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        device::books::restore(afc, &j.books, &j.books_plan())?;
        let (request, retained) = j
            .plan
            .request(step, indices, &j.books)
            .map_err(|_| invalid("customization_request"))?;
        device::books::stage_customization_request(
            afc,
            &j.books,
            &j.books_plan(),
            &request,
            cancelled,
        )?;
        let assets: Vec<_> = j
            .plan
            .transfers(step, indices)
            .map_err(|_| invalid("customization_transfers"))?
            .into_iter()
            .map(|(asset_id, asset_path)| airtraffic::handshake::SyncAsset {
                asset_id,
                asset_path,
                retained_ids: retained.clone(),
            })
            .collect();
        let (_, service) =
            device::open_verified_service(self.provider, self.selected, "com.apple.atc")?;
        let report = airtraffic::handshake::HandshakeClient {
            transport: Bridge(service),
        }
        .synchronize_batch_checked(
            airtraffic::handshake::SyncOptions {
                library_id: &j.plan.transaction,
                grappa: Some(self.token),
                timeout: Duration::from_secs(self.timeout),
                cancelled,
            },
            &assets,
            &mut || device::books::original_content_unchanged(afc, &j.books).unwrap_or(false),
            emit,
        );
        let ok = report.failure.is_none() && report.state == airtraffic::State::Finished;
        emit(&report);
        if ok {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Native, "customization_sync_failed"))
        }
    }
    pub(crate) fn restore(
        &self,
        afc: &mut device::LinuxAfc,
        j: &mut Journal,
        file: &mut FileJournal<'_>,
    ) -> Result<()> {
        if !j.export_started || j.restored {
            return Ok(());
        }
        // Original moves may have completed before the previous process persisted the byte snapshot.
        for i in 0..j.plan.leaves.len() {
            if let Some(data) = read_slot(afc, &j.plan.slot("original", i))? {
                if j.originals.get(&i).is_some_and(|old| old != &data) {
                    return Err(Error::new(
                        ErrorKind::Conflict,
                        "customization_original_changed",
                    ));
                }
                j.originals.insert(i, data);
            }
        }
        file.save(j)?;
        let active: Vec<_> = j.originals.keys().copied().collect();
        if active.is_empty() {
            j.restored = true;
            return file.save(j);
        }
        // A failed previous attempt may have moved an unrelated concurrent edit. Return
        // that exact byte preimage before attempting any new restore, even after a restart.
        if matches!(j.plan.target, Target::WalletArtwork(_)) {
            for round in 0..j.rollback_round {
                let mut unknown = vec![];
                for &i in &active {
                    if let Some(bytes) =
                        read_slot(afc, &j.plan.slot(&format!("rollback-{round}"), i))?
                        && bytes != j.originals[&i]
                        && bytes != j.payloads[i]
                    {
                        unknown.push(i);
                    }
                }
                if !unknown.is_empty() {
                    self.sync(afc, j, Step::ReturnRollback(round), &unknown, &|| false)?;
                    return Err(Error::new(
                        ErrorKind::Conflict,
                        "unrelated_card_changes_preserved",
                    ));
                }
            }
        }
        if j.rollback_round >= 8 {
            return Err(invalid("customization_recovery_limit"));
        }
        let round = j.rollback_round;
        j.rollback_round += 1;
        file.save(j)?;
        let group = format!("rollback-{round}");
        afc.mkdir(&format!("{}/{group}", j.plan.work()))?;
        // Each recovery attempt gets a fresh destination; an interrupted prior attempt cannot
        // make a successful no-op look like a new export from the protected target.
        let exported = self.sync(afc, j, Step::Rollback(round), &active, &|| false);
        let mut unknown = vec![];
        for &i in &active {
            if let Some(current) = read_slot(afc, &j.plan.slot(&group, i))?
                && !matches!(j.plan.target, Target::WalletCache { .. })
                && current != j.originals[&i]
                && current != j.payloads[i]
            {
                unknown.push(i);
            }
        }
        if !unknown.is_empty() {
            self.sync(afc, j, Step::ReturnRollback(round), &unknown, &|| false)?;
            return Err(Error::new(
                ErrorKind::Conflict,
                "unrelated_card_changes_preserved",
            ));
        }
        exported?;
        for &i in &active {
            let path = j.plan.slot("restore", i);
            if let Some(bytes) = read_slot(afc, &path)? {
                if bytes != j.originals[&i] {
                    return Err(Error::new(
                        ErrorKind::Conflict,
                        "customization_restore_slot_changed",
                    ));
                }
            } else {
                afc.write(&path, &j.originals[&i])?;
            }
            if afc.read(&path, MAX_FILE_BYTES)? != j.originals[&i] {
                return Err(invalid("customization_restore_stage_verify"));
            }
        }
        self.sync(afc, j, Step::RestoreLocal, &active, &|| false)?;
        for &i in &active {
            if read_slot(afc, &j.plan.slot("restore", i))?.is_some() {
                return Err(invalid("customization_restore_move_not_completed"));
            }
        }
        if j.rollback_round >= 8 {
            return Err(invalid("customization_recovery_limit"));
        }
        let check_round = j.rollback_round;
        j.rollback_round += 1;
        file.save(j)?;
        let check_group = format!("rollback-{check_round}");
        afc.mkdir(&format!("{}/{check_group}", j.plan.work()))?;
        self.sync(afc, j, Step::Rollback(check_round), &active, &|| false)?;
        for &i in &active {
            if read_slot(afc, &j.plan.slot(&check_group, i))?.as_ref() != Some(&j.originals[&i]) {
                return Err(invalid("card_restore_readback_mismatch"));
            }
        }
        self.sync(afc, j, Step::ReturnRollback(check_round), &active, &|| {
            false
        })?;
        for &i in &active {
            if read_slot(afc, &j.plan.slot(&check_group, i))?.is_some() {
                return Err(invalid("card_restore_verified_return_failed"));
            }
        }
        j.restored = true;
        file.save(j)?;
        emit(
            &json!({"event":"card_originals_restored","files":active.len(),"staged_bytes_verified":true,"moves_verified":true,"readback_verified":true}),
        );
        Ok(())
    }
}
pub(crate) fn read_slot(afc: &mut impl AfcAccess, path: &str) -> Result<Option<Vec<u8>>> {
    match afc.stat(path) {
        Err(e) if e.kind == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
        Ok(info) => {
            if info.kind != device::FileKind::File || info.size > MAX_FILE_BYTES as u64 {
                return Err(invalid("customization_slot_type_or_size"));
            }
            let data = afc.read(path, MAX_FILE_BYTES)?;
            if data.len() as u64 != info.size {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "customization_slot_changed",
                ));
            }
            Ok(Some(data))
        }
    }
}
pub(crate) fn finish(afc: &mut device::LinuxAfc, j: &Journal) -> Result<()> {
    let books = device::books::restore(afc, &j.books, &j.books_plan());
    let staging = afc.staging_cleanup(&j.cleanup_plan());
    emit(
        &json!({"event":"customization_cleanup","books_restored":books.is_ok(),"staging_removed":staging.is_ok(),"books_error":books.as_ref().err(),"staging_error":staging.as_ref().err()}),
    );
    books?;
    staging
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_tampering_and_out_of_scope_originals_are_rejected() {
        let mut j = Journal {
            schema: 1,
            device: "a".repeat(64),
            plan: Plan {
                transaction: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
                target: Target::WalletArtwork("AAoUHigyPEZQWmRueIKMlqCqtL4=".into()),
                leaves: vec!["cardBackgroundCombined@3x.png".into()],
            },
            books: BooksSnapshot {
                entries: Default::default(),
            },
            originals: BTreeMap::from([(0, vec![1, 2])]),
            payloads: vec![vec![3, 4]],
            export_started: true,
            apply_started: false,
            restored: false,
            rollback_round: 0,
            checksum: String::new(),
        };
        j.bytes().unwrap();
        assert!(j.validate().is_ok());
        j.originals.get_mut(&0).unwrap().push(5);
        assert!(j.validate().is_err());
        j.bytes().unwrap();
        j.originals.insert(2, vec![]);
        j.checksum = j.checksum().unwrap();
        assert!(j.validate().is_err());
    }
}
