//! Recoverable two-file activation. Callers hold installation/profile locks.
use anyhow::{Context, Result, bail};
use infra_local_state::{LocalConfig, read_config, write_config};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const JOURNAL: &str = ".infra-activation.json";
pub fn pending(install: &Path) -> bool {
    install.join(JOURNAL).exists()
}
pub fn validate_recovery_profile(install: &Path, state: &Path) -> Result<()> {
    let record: Journal = serde_json::from_slice(&fs::read(install.join(JOURNAL))?)?;
    if record.version != 1 || record.state != fs::canonicalize(state)? {
        bail!("activation journal belongs to another profile/version; preserve recovery files");
    }
    Ok(())
}
pub const CLI: &str = if cfg!(windows) { "infra.exe" } else { "infra" };
pub const AGENT: &str = if cfg!(windows) {
    "infra-agent.exe"
} else {
    "infra-agent"
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PairHash {
    pub cli: Option<String>,
    pub agent: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u8,
    id: Uuid,
    state: PathBuf,
    old: PairHash,
    new: PairHash,
    config: Option<LocalConfig>,
}
pub struct PairTransaction {
    install: PathBuf,
    state: PathBuf,
    record: Option<Journal>,
}

pub fn hash(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("read binary {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn optional_hash(path: &Path) -> Result<Option<String>> {
    if path.try_exists()? {
        Ok(Some(hash(path)?))
    } else {
        Ok(None)
    }
}
pub fn pair_hash(install: &Path) -> Result<PairHash> {
    Ok(PairHash {
        cli: optional_hash(&install.join(CLI))?,
        agent: optional_hash(&install.join(AGENT))?,
    })
}
fn owned(install: &Path, id: Uuid, part: &str) -> PathBuf {
    install.join(format!(".infra-{id}-{part}"))
}
fn copy_synced(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)?;
    // Windows FlushFileBuffers requires a handle opened for writing.
    fs::OpenOptions::new().write(true).open(to)?.sync_all()?;
    Ok(())
}
fn write_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
impl PairTransaction {
    pub fn prepare(install: &Path, state: &Path, source: &Path) -> Result<Self> {
        let install = fs::canonicalize(install)?;
        let state = fs::canonicalize(state)?;
        if install.join(JOURNAL).exists() {
            bail!(
                "activation_recovery_required: finish the previous transaction before activation"
            );
        }
        for name in [CLI, AGENT] {
            if fs::metadata(source.join(name))?.len() == 0 {
                bail!("empty candidate binary: {name}");
            }
        }
        let old = pair_hash(&install)?;
        let new = pair_hash(source)?;
        if new.cli.is_none() || new.agent.is_none() {
            bail!("both candidate binaries are required");
        }
        if old == new {
            return Ok(Self {
                install,
                state,
                record: None,
            });
        }
        let record = Journal {
            version: 1,
            id: Uuid::new_v4(),
            state: state.clone(),
            old,
            new,
            config: read_config(&state)?,
        };
        let staging = (|| -> Result<()> {
            for (name, part, expected, old_hash) in [
                (CLI, "cli", &record.new.cli, &record.old.cli),
                (AGENT, "agent", &record.new.agent, &record.old.agent),
            ] {
                let stage = owned(&install, record.id, &format!("{part}.stage"));
                copy_synced(&source.join(name), &stage)?;
                if Some(hash(&stage)?) != *expected {
                    bail!("candidate changed while staging");
                }
                if let Some(expected) = old_hash {
                    let backup = owned(&install, record.id, &format!("{part}.backup"));
                    copy_synced(&install.join(name), &backup)?;
                    if hash(&backup)? != *expected {
                        bail!("installed binary changed while backing up");
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = staging {
            for part in ["cli.stage", "agent.stage", "cli.backup", "agent.backup"] {
                let _ = fs::remove_file(owned(&install, record.id, part));
            }
            return Err(error);
        }
        write_new(&install.join(JOURNAL), &record)?;
        sync_directory(&install)?;
        Ok(Self {
            install,
            state,
            record: Some(record),
        })
    }
    pub fn changed(&self) -> bool {
        self.record.is_some()
    }
    pub fn has_previous(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|r| r.old.cli.is_some() || r.old.agent.is_some())
    }
    pub fn fully_active(&self) -> bool {
        self.record
            .as_ref()
            .is_none_or(|r| pair_hash(&self.install).ok().as_ref() == Some(&r.new))
    }
    pub fn activate(&self) -> Result<()> {
        if let Some(r) = &self.record {
            fs::rename(
                owned(&self.install, r.id, "agent.stage"),
                self.install.join(AGENT),
            )
            .context("activate agent")?;
            fs::rename(
                owned(&self.install, r.id, "cli.stage"),
                self.install.join(CLI),
            )
            .context("activate CLI")?;
            sync_directory(&self.install)?;
        }
        Ok(())
    }
    pub fn commit(&self) -> Result<()> {
        if let Some(r) = &self.record {
            if pair_hash(&self.install)? != r.new {
                bail!(
                    "activation_changed: candidate pair no longer matches; recovery files retained"
                );
            }
            write_new(&owned(&self.install, r.id, "committed"), &true)?;
            sync_directory(&self.install)?;
            self.cleanup().context("activation_committed_cleanup_pending: pair retained; rerun installer to finish cleanup")?;
        }
        Ok(())
    }
    pub fn rollback(&self) -> Result<()> {
        let Some(r) = &self.record else {
            return Ok(());
        };
        let mut failures = Vec::new();
        for (name, part, old, new) in [
            (CLI, "cli", &r.old.cli, &r.new.cli),
            (AGENT, "agent", &r.old.agent, &r.new.agent),
        ] {
            let result = (|| -> Result<()> {
                let target = self.install.join(name);
                let current = optional_hash(&target)?;
                if current == *old {
                    return Ok(());
                }
                if current.is_some() && current != *new {
                    bail!("{name} changed outside activation; refusing overwrite");
                }
                if let Some(expected) = old {
                    let backup = owned(&self.install, r.id, &format!("{part}.backup"));
                    if hash(&backup)? != *expected {
                        bail!("{name} backup integrity failed");
                    }
                    let restore = owned(&self.install, r.id, &format!("{part}.restore"));
                    copy_synced(&backup, &restore)?;
                    fs::rename(restore, target)?;
                } else if target.exists() {
                    fs::remove_file(target)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                failures.push(format!("{name}: {error:#}"));
            }
        }
        if !failures.is_empty() {
            bail!(
                "rollback_failed: {}; journal and backups retained for recovery",
                failures.join("; ")
            );
        }
        if let Some(config) = &r.config {
            write_config(&self.state, config)?;
        }
        sync_directory(&self.install)?;
        self.cleanup()
    }
    fn cleanup(&self) -> Result<()> {
        let Some(r) = &self.record else {
            return Ok(());
        };
        for part in [
            "cli.stage",
            "agent.stage",
            "cli.backup",
            "agent.backup",
            "cli.restore",
            "agent.restore",
        ] {
            let path = owned(&self.install, r.id, part);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        // Journal goes first: an orphan commit marker is harmless. Removing the
        // marker first could make an interrupted cleanup appear uncommitted.
        fs::remove_file(self.install.join(JOURNAL))?;
        let _ = fs::remove_file(owned(&self.install, r.id, "committed"));
        sync_directory(&self.install)
    }
    pub fn recover(install: &Path, state: &Path) -> Result<bool> {
        let path = install.join(JOURNAL);
        if !path.try_exists()? {
            return Ok(false);
        }
        let record: Journal = serde_json::from_slice(&fs::read(path)?)
            .context("activation journal is unreadable; preserve recovery files")?;
        if record.version != 1 || record.state != fs::canonicalize(state)? {
            bail!("activation journal belongs to another profile/version; preserve recovery files");
        }
        let tx = Self {
            install: fs::canonicalize(install)?,
            state: record.state.clone(),
            record: Some(record),
        };
        let r = tx.record.as_ref().unwrap();
        if owned(&tx.install, r.id, "committed").exists() {
            if pair_hash(&tx.install)? != r.new {
                bail!("committed binary pair changed; preserve recovery files");
            }
            tx.cleanup()?;
        } else {
            tx.rollback()?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("bin with spaces");
        let state = dir.path().join("state");
        let source = dir.path().join("source");
        for path in [&install, &state, &source] {
            fs::create_dir_all(path).unwrap();
        }
        for (folder, prefix) in [(&install, "old"), (&source, "new")] {
            fs::write(folder.join(CLI), format!("{prefix}-cli")).unwrap();
            fs::write(folder.join(AGENT), format!("{prefix}-agent")).unwrap();
        }
        fs::write(state.join("identity.json"), b"preserve-fixture-identity").unwrap();
        (dir, install, state, source)
    }
    #[test]
    fn second_swap_interruption_restores_both_images_and_keeps_identity() {
        let (_dir, install, state, source) = fixture();
        let before = pair_hash(&install).unwrap();
        let tx = PairTransaction::prepare(&install, &state, &source).unwrap();
        let id = tx.record.as_ref().unwrap().id;
        fs::rename(owned(&install, id, "agent.stage"), install.join(AGENT)).unwrap();
        drop(tx);
        assert!(PairTransaction::recover(&install, &state).unwrap());
        assert_eq!(pair_hash(&install).unwrap(), before);
        assert_eq!(
            fs::read(state.join("identity.json")).unwrap(),
            b"preserve-fixture-identity"
        );
    }
    #[test]
    fn failed_restore_retains_evidence_and_can_be_retried() {
        let (_dir, install, state, source) = fixture();
        let old = pair_hash(&install).unwrap();
        let tx = PairTransaction::prepare(&install, &state, &source).unwrap();
        tx.activate().unwrap();
        let backup = owned(&install, tx.record.as_ref().unwrap().id, "cli.backup");
        let saved = fs::read(&backup).unwrap();
        fs::write(&backup, b"corrupted").unwrap();
        assert!(tx.rollback().is_err());
        assert!(install.join(JOURNAL).exists());
        assert!(backup.exists());
        fs::write(backup, saved).unwrap();
        assert!(PairTransaction::recover(&install, &state).unwrap());
        assert_eq!(pair_hash(&install).unwrap(), old);
    }
    #[test]
    fn commit_recovery_keeps_the_verified_pair_and_same_pair_is_noop() {
        let (_dir, install, state, source) = fixture();
        let tx = PairTransaction::prepare(&install, &state, &source).unwrap();
        tx.activate().unwrap();
        write_new(
            &owned(&install, tx.record.as_ref().unwrap().id, "committed"),
            &true,
        )
        .unwrap();
        drop(tx);
        PairTransaction::recover(&install, &state).unwrap();
        assert_eq!(pair_hash(&install).unwrap(), pair_hash(&source).unwrap());
        assert!(
            !PairTransaction::prepare(&install, &state, &source)
                .unwrap()
                .changed()
        );
    }
    #[test]
    fn incomplete_candidate_or_foreign_profile_cannot_modify_install() {
        let (_dir, install, state, source) = fixture();
        let old = pair_hash(&install).unwrap();
        fs::remove_file(source.join(AGENT)).unwrap();
        assert!(PairTransaction::prepare(&install, &state, &source).is_err());
        assert_eq!(pair_hash(&install).unwrap(), old);
        fs::write(source.join(AGENT), b"new-agent").unwrap();
        let _tx = PairTransaction::prepare(&install, &state, &source).unwrap();
        let other = state.join("other");
        fs::create_dir_all(&other).unwrap();
        assert!(PairTransaction::recover(&install, &other).is_err());
        assert!(install.join(JOURNAL).exists());
    }
}
