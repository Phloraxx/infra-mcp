use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let pkg_version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by cargo");
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain"])
        .map(|output| !output.trim().is_empty())
        .unwrap_or(false);
    let suffix = if dirty { ".dirty" } else { "" };
    let full = format!("{pkg_version}+{commit}{suffix}");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let contents = [
        format!("pub(crate) const VERSION: &str = \"{pkg_version}\";"),
        format!("pub(crate) const COMMIT: &str = \"{commit}\";"),
        format!("pub(crate) const VERSION_FULL: &str = \"{full}\";"),
        String::new(),
    ]
    .join(
        "
",
    );
    fs::write(out_dir.join("infra_version.rs"), contents)
        .expect("write generated version constants");
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let trimmed = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
