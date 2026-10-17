//! Small cross-platform primitives used by the headless agent.

pub mod secret_store;

use anyhow::Context;
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct PlatformIdentity {
    pub platform: &'static str,
    pub arch: &'static str,
    pub hostname: String,
    pub boot_id: String,
}

pub fn identity() -> PlatformIdentity {
    PlatformIdentity {
        platform: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        hostname: hostname(),
        boot_id: boot_id().unwrap_or_else(|_| "unknown".into()),
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

fn boot_id() -> anyhow::Result<String> {
    #[cfg(target_os = "linux")]
    {
        Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let boot_time = sysinfo::System::boot_time();
        if boot_time > 0 {
            Ok(format!("boot-time-{boot_time}"))
        } else {
            Ok(format!("process-{}", std::process::id()))
        }
    }
}

pub fn ensure_private_dir(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("create state directory {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_usable() {
        let identity = identity();
        assert!(!identity.platform.is_empty());
        assert!(!identity.arch.is_empty());
        assert!(!identity.hostname.is_empty());
        assert!(!identity.boot_id.is_empty());
    }
}
