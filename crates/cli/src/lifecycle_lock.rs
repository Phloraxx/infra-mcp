//! OS-owned locks. Never unlink their files: doing so would permit split locks.
use anyhow::{Context, Result, anyhow};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

pub struct LifecycleLocks {
    _files: Vec<File>,
}
impl LifecycleLocks {
    pub fn acquire(install: &Path, state: &Path) -> Result<Self> {
        fs::create_dir_all(install)?;
        let install = fs::canonicalize(install)?;
        let state = if state.try_exists()? {
            fs::canonicalize(state)?
        } else {
            let absolute = std::path::absolute(state)?;
            let parent = absolute
                .parent()
                .ok_or_else(|| anyhow!("profile parent missing"))?;
            fs::create_dir_all(parent)?;
            fs::canonicalize(parent)?.join(
                absolute
                    .file_name()
                    .ok_or_else(|| anyhow!("profile name missing"))?,
            )
        };
        if state.parent().is_none() || install.starts_with(&state) {
            return Err(anyhow!(
                "installation_layout_invalid: keep binaries outside the removable profile and do not use filesystem root as state"
            ));
        }
        let mut state_name = state
            .file_name()
            .ok_or_else(|| anyhow!("state directory cannot be filesystem root"))?
            .to_os_string();
        state_name.push(".infra-lifecycle.lock");
        let state_lock = state.parent().unwrap().join(state_name);
        let mut paths = vec![install.join(".infra-install.lock"), state_lock];
        paths.sort();
        paths.dedup();
        let mut files = Vec::new();
        for path in paths {
            files.push(acquire_file(&path)?);
        }
        infra_local_state::ensure_state_dir(&state)?;
        Ok(Self { _files: files })
    }
}
fn acquire_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open lifecycle lock {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(anyhow!(
            "installation_busy: another Infra lifecycle operation owns this installation or profile; wait for it to finish. Lock files must not be deleted."
        )),
        Err(error) => Err(anyhow!("lifecycle_lock_failed: {error}")),
    }
}
pub fn current_install_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("CLI directory missing"))?
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_lock_holder() {
        let Some(dir) = std::env::var_os("INFRA_TEST_LOCK_ROOT") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let _guard = LifecycleLocks::acquire(&dir.join("bin"), &dir.join("state")).unwrap();
        fs::write(dir.join("ready"), b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    #[test]
    fn process_exit_releases_locks_without_deleting_lock_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lifecycle_lock::tests::child_lock_holder"])
            .env("INFRA_TEST_LOCK_ROOT", dir.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !dir.path().join("ready").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let ready = dir.path().join("ready").exists();
        let blocked =
            LifecycleLocks::acquire(&dir.path().join("bin"), &dir.path().join("state")).is_err();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(ready && blocked);
        assert!(
            LifecycleLocks::acquire(&dir.path().join("bin"), &dir.path().join("state")).is_ok()
        );
    }
    #[test]
    fn guards_exclude_shared_install_or_profile_and_survive_profile_removal() {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("bin");
        let state = dir.path().join("state");
        let first = LifecycleLocks::acquire(&install, &state).unwrap();
        assert!(LifecycleLocks::acquire(&install, &dir.path().join("other-state")).is_err());
        assert!(LifecycleLocks::acquire(&dir.path().join("other-bin"), &state).is_err());
        fs::remove_dir_all(&state).unwrap();
        assert!(LifecycleLocks::acquire(&install, &state).is_err());
        assert!(
            !state.exists(),
            "contending operation must not recreate a removed profile"
        );
        drop(first);
        // A parallel test may fork while the guard exists; inherited handles
        // close on exec. Require bounded release, not zero scheduling delay.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let _reacquired = loop {
            match LifecycleLocks::acquire(&install, &state) {
                Ok(guard) => break guard,
                Err(error)
                    if error.to_string().starts_with("installation_busy:")
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("lock did not release after dropping owner: {error:#}"),
            }
        };
    }
}
