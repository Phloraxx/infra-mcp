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
    // HOSTNAME and /etc/hostname are Unix conventions, not Windows ones.
    // Windows provides COMPUTERNAME. Prefer native OS discovery everywhere,
    // with the environment as a fallback for constrained/headless runtimes.
    #[cfg(windows)]
    if let Ok(name) = std::env::var("COMPUTERNAME")
        && !name.trim().is_empty()
    {
        return name.trim().to_string();
    }
    sysinfo::System::host_name()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| !name.trim().is_empty())
        })
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
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

    #[cfg(windows)]
    #[test]
    fn windows_uses_actual_computer_name() {
        let os_name = std::env::var("COMPUTERNAME").expect("Windows provides COMPUTERNAME");
        assert!(!os_name.trim().is_empty());
        assert_eq!(identity().hostname, os_name);
    }

    #[test]
    fn identity_is_usable() {
        let identity = identity();
        assert!(!identity.platform.is_empty());
        assert!(!identity.arch.is_empty());
        assert!(!identity.hostname.is_empty());
        assert!(!identity.boot_id.is_empty());
    }
}
