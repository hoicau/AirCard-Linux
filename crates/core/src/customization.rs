//! Typed theme destinations and transaction mappings. No platform or filesystem calls.
use crate::{
    Error,
    staging::{StagingPlan, validate_transaction},
};
use plist::{Dictionary, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    WalletArtwork(String),
    WalletCache { hash: String, extension: String },
}
impl Target {
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::WalletArtwork(hash) | Self::WalletCache { hash, .. } => {
                crate::safe_leaf(hash)?;
                if !crate::scanner::is_valid_card_hash(hash) {
                    return Err(Error::UnsafePath);
                }
                if let Self::WalletCache { extension, .. } = self
                    && !["cache", "pkcache"].contains(&extension.as_str())
                {
                    return Err(Error::UnsafePath);
                }
                Ok(())
            }
        }
    }
    pub fn directory(&self) -> String {
        match self {
            Self::WalletArtwork(hash) => format!("/var/mobile/Library/Passes/Cards/{hash}.pkpass"),
            Self::WalletCache { hash, extension } => {
                format!("/var/mobile/Library/Passes/Cards/{hash}.{extension}")
            }
        }
    }
    pub fn leaf_allowed(&self, leaf: &str) -> bool {
        if crate::safe_leaf(leaf).is_err() {
            return false;
        }
        match self {
            Self::WalletArtwork(_) => [
                "cardBackgroundCombined@3x.png",
                "cardBackgroundCombined@2x.png",
                "cardBackgroundCombined.pdf",
            ]
            .contains(&leaf),
            Self::WalletCache { .. } => ["FrontFace", "PlaceHolder", "Preview"].contains(&leaf),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub transaction: String,
    pub target: Target,
    pub leaves: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Step {
    Link,
    ExportOriginals,
    Install,
    ExportInstalled,
    ReturnInstalled,
    RestoreOriginals,
    RestoreLocal,
    Rollback(u8),
    ReturnRollback(u8),
}
impl Plan {
    pub fn validate(&self) -> Result<(), Error> {
        validate_transaction(&self.transaction)?;
        self.target.validate()?;
        if self.leaves.is_empty() || self.leaves.len() > 64 {
            return Err(Error::Limit);
        }
        let mut seen = BTreeSet::new();
        for leaf in &self.leaves {
            if !self.target.leaf_allowed(leaf) || !seen.insert(leaf) {
                return Err(Error::UnsafePath);
            }
        }
        Ok(())
    }
    pub fn source(&self) -> String {
        format!("AirCard-Linux-Theme-{}", self.transaction)
    }
    pub fn link(&self) -> String {
        format!("AirCard-Linux-Link-{}", self.transaction)
    }
    pub fn work(&self) -> String {
        format!("AirCard-Linux-Work-{}", self.transaction)
    }
    pub fn slot(&self, group: &str, index: usize) -> String {
        format!("{}/{group}/{index}", self.work())
    }
    pub fn archive(&self, payloads: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
        self.validate()?;
        if payloads.len() != self.leaves.len()
            || payloads.iter().any(|p| p.len() > 16 * 1024 * 1024)
            || payloads.iter().map(Vec::len).sum::<usize>() > 64 * 1024 * 1024
        {
            return Err(Error::Limit);
        }
        let items: Vec<_> = self
            .leaves
            .iter()
            .zip(payloads)
            .map(|(leaf, payload)| (leaf.as_str(), payload.as_slice()))
            .collect();
        crate::staging::build_streaming_zip_archive_multi(&self.target.directory(), &items)
    }
    pub fn transfers(&self, step: Step, indices: &[usize]) -> Result<Vec<(String, String)>, Error> {
        self.validate()?;
        if matches!(step,Step::Rollback(round)|Step::ReturnRollback(round) if round>=8) {
            return Err(Error::Limit);
        }
        let mut seen = BTreeSet::new();
        if indices
            .iter()
            .any(|i| *i >= self.leaves.len() || !seen.insert(*i))
        {
            return Err(Error::UnsafePath);
        }
        if matches!(step, Step::Link) {
            return Ok(vec![(
                format!("../../{}/p0/p1/p2/link", self.source()),
                self.link(),
            )]);
        }
        if indices.is_empty() {
            return Err(Error::Limit);
        }
        Ok(indices
            .iter()
            .map(|&i| {
                let leaf = &self.leaves[i];
                let target_id = format!(
                    "../../../{}/{}",
                    self.target
                        .directory()
                        .strip_prefix("/var/mobile/")
                        .expect("typed target"),
                    leaf
                );
                let destination = format!("{}/{leaf}", self.link());
                match step {
                    Step::ExportOriginals => (target_id, self.slot("original", i)),
                    Step::Install => (format!("../../{}/payload_{i}", self.source()), destination),
                    Step::ExportInstalled => (target_id, self.slot("verified", i)),
                    Step::ReturnInstalled => {
                        (format!("../../{}", self.slot("verified", i)), destination)
                    }
                    Step::RestoreOriginals => {
                        (format!("../../{}", self.slot("original", i)), destination)
                    }
                    Step::RestoreLocal => {
                        (format!("../../{}", self.slot("restore", i)), destination)
                    }
                    Step::Rollback(round) => {
                        (target_id, self.slot(&format!("rollback-{round}"), i))
                    }
                    Step::ReturnRollback(round) => (
                        format!("../../{}", self.slot(&format!("rollback-{round}"), i)),
                        destination,
                    ),
                    Step::Link => unreachable!(),
                }
            })
            .collect())
    }
    pub fn request(
        &self,
        step: Step,
        indices: &[usize],
        snapshot: &crate::books::BooksSnapshot,
    ) -> Result<(Vec<u8>, BTreeSet<String>), Error> {
        let base = StagingPlan {
            transaction: self.transaction.clone(),
        };
        let (bytes, retained) = base.request(snapshot)?;
        let mut value = crate::decode_binary(&bytes)?;
        let rows = value
            .as_dictionary_mut()
            .and_then(|d| d.get_mut("Books"))
            .and_then(Value::as_array_mut)
            .ok_or(Error::UnsafePath)?;
        rows.pop();
        let transfers = self.transfers(step, indices)?;
        if rows.len() + transfers.len() > 128 {
            return Err(Error::Limit);
        }
        for (n, (id, _)) in transfers.into_iter().enumerate() {
            rows.push(Value::Dictionary(Dictionary::from_iter([
                ("Persistent ID", Value::String(id)),
                ("Item ID", Value::String((n + 1).to_string())),
                ("DSID", Value::String("1".into())),
            ])));
        }
        Ok((crate::encode_binary(&value)?, retained))
    }
}
/// Validate the narrow on-wire mapping; arbitrary traversal never becomes a normal Book ID.
pub fn is_transfer(id: &str, destination: &str) -> bool {
    if let Some(rest) = destination.strip_prefix("AirCard-Linux-Work-") {
        let parts: Vec<_> = rest.split('/').collect();
        if parts.len() != 3
            || validate_transaction(parts[0]).is_err()
            || !(["original", "verified"].contains(&parts[1])
                || parts[1]
                    .strip_prefix("rollback-")
                    .is_some_and(|n| n.len() == 1 && n.parse::<u8>().is_ok_and(|n| n < 8)))
            || !index(parts[2])
        {
            return false;
        }
        let Some(path) = id.strip_prefix("../../../Library/") else {
            return false;
        };
        if let Some((directory, leaf)) = path.rsplit_once('/')
            && let Some(card) = directory
                .strip_prefix("Passes/Cards/")
                .and_then(|v| v.rsplit_once('.'))
        {
            let target = if card.1 == "pkpass" {
                Target::WalletArtwork(card.0.into())
            } else {
                Target::WalletCache {
                    hash: card.0.into(),
                    extension: card.1.into(),
                }
            };
            return target.validate().is_ok() && target.leaf_allowed(leaf);
        }
    }
    if let Some((transaction, leaf)) = destination
        .strip_prefix("AirCard-Linux-Link-")
        .and_then(|s| s.split_once('/'))
    {
        if validate_transaction(transaction).is_err()
            || crate::safe_leaf(leaf).is_err()
            || !([
                "cardBackgroundCombined@3x.png",
                "cardBackgroundCombined@2x.png",
                "cardBackgroundCombined.pdf",
                "FrontFace",
                "PlaceHolder",
                "Preview",
            ]
            .contains(&leaf))
        {
            return false;
        }
        if let Some(n) =
            id.strip_prefix(&format!("../../AirCard-Linux-Theme-{transaction}/payload_"))
        {
            return index(n);
        }
        if let Some(rest) = id.strip_prefix(&format!("../../AirCard-Linux-Work-{transaction}/"))
            && let Some((group, n)) = rest.split_once('/')
        {
            return (["original", "verified", "restore"].contains(&group)
                || group
                    .strip_prefix("rollback-")
                    .is_some_and(|r| r.len() == 1 && r.parse::<u8>().is_ok_and(|r| r < 8)))
                && index(n);
        }
    }
    false
}
fn index(s: &str) -> bool {
    s.parse::<usize>()
        .is_ok_and(|n| n < 64 && n.to_string() == s)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_paths_match_wire_validation_and_reject_unrelated_data() {
        let p = Plan {
            transaction: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            target: Target::WalletArtwork("AAoUHigyPEZQWmRueIKMlqCqtL4=".into()),
            leaves: vec![
                "cardBackgroundCombined@3x.png".into(),
                "cardBackgroundCombined@2x.png".into(),
            ],
        };
        for step in [
            Step::ExportOriginals,
            Step::Install,
            Step::ExportInstalled,
            Step::ReturnInstalled,
            Step::RestoreOriginals,
            Step::RestoreLocal,
            Step::Rollback(0),
            Step::ReturnRollback(7),
        ] {
            for (id, path) in p.transfers(step, &[0, 1]).unwrap() {
                assert!(is_transfer(&id, &path));
                assert!(!is_transfer(&(id + "/../secret"), &path));
            }
        }
        assert!(!is_transfer(
            "../../../Library/SMS/sms.db",
            &p.slot("original", 0)
        ));
        assert!(!is_transfer(
            "../../../Library/Caches/TelephonyUI-10/../secret",
            &p.slot("original", 0)
        ));
        assert!(p.transfers(Step::Install, &[0, 0]).is_err());
        assert!(p.transfers(Step::Install, &[2]).is_err());
    }
}
