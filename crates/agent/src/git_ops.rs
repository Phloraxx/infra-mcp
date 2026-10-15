use crate::command_runner::{require_success, run};
use infra_protocol::{GitAction, GitRequest, OperationResult};
use serde::Serialize;
use std::{path::Path, process::Command, time::Duration};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct CommitView {
    hash: String,
    author: String,
    date: String,
    subject: String,
}

pub fn handle(request: GitRequest) -> OperationResult {
    let result = match request.action {
        GitAction::Status { repo } => status(&repo),
        GitAction::Diff { repo, staged } => diff(&repo, staged),
        GitAction::Log { repo, limit } => log(&repo, limit),
        GitAction::Branches { repo } => branches(&repo),
        GitAction::Remotes { repo } => remotes(&repo),
        GitAction::Fetch { repo, remote } => fetch(&repo, remote.as_deref()),
        GitAction::Pull {
            repo,
            remote,
            branch,
        } => pull(&repo, remote.as_deref(), branch.as_deref()),
        GitAction::Push {
            repo,
            remote,
            branch,
        } => push(&repo, remote.as_deref(), branch.as_deref()),
    };
    match result {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed { error },
    }
}

fn git(
    repo: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<crate::command_runner::CommandOutput, String> {
    let path = Path::new(repo);
    if !path.is_dir() {
        return Err("repo must be an existing directory".into());
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never");
    require_success(run(&mut command, timeout, OUTPUT_LIMIT)?)
}

fn status(repo: &str) -> Result<serde_json::Value, String> {
    let output = git(
        repo,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
        ],
        Duration::from_secs(15),
    )?;
    let mut branch = serde_json::Map::new();
    let mut changes = Vec::new();
    for line in output.stdout.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some((key, value)) = rest.split_once(' ') {
                branch.insert(key.to_string(), value.into());
            }
        } else if !line.is_empty() {
            changes.push(line.to_string());
        }
    }
    Ok(serde_json::json!({
        "repo":repo,
        "branch":branch,
        "changes":changes,
        "clean":changes.is_empty(),
        "truncated":output.truncated
    }))
}

fn diff(repo: &str, staged: bool) -> Result<serde_json::Value, String> {
    let args = if staged {
        vec!["diff", "--cached", "--no-ext-diff", "--no-color"]
    } else {
        vec!["diff", "--no-ext-diff", "--no-color"]
    };
    let output = git(repo, &args, Duration::from_secs(20))?;
    Ok(
        serde_json::json!({"repo":repo,"staged":staged,"diff":output.stdout,"truncated":output.truncated}),
    )
}

fn log(repo: &str, limit: Option<usize>) -> Result<serde_json::Value, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200).to_string();
    let format = "%H%x09%an%x09%aI%x09%s";
    let output = git(
        repo,
        &[
            "log",
            "--no-color",
            "-n",
            &limit,
            &format!("--pretty=format:{format}"),
        ],
        Duration::from_secs(20),
    )?;
    let commits = output
        .stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            Some(CommitView {
                hash: parts.next()?.into(),
                author: parts.next()?.into(),
                date: parts.next()?.into(),
                subject: parts.next().unwrap_or_default().into(),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"repo":repo,"commits":commits,"truncated":output.truncated}))
}

fn branches(repo: &str) -> Result<serde_json::Value, String> {
    let output = git(
        repo,
        &[
            "branch",
            "--all",
            "--no-color",
            "--format=%(refname:short)%09%(HEAD)",
        ],
        Duration::from_secs(15),
    )?;
    let branches = output
        .stdout
        .lines()
        .map(|line| {
            let (name, head) = line.split_once('\t').unwrap_or((line, ""));
            serde_json::json!({"name":name,"current":head.trim()=="*"})
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"repo":repo,"branches":branches,"truncated":output.truncated}))
}

fn remotes(repo: &str) -> Result<serde_json::Value, String> {
    let output = git(repo, &["remote", "-v"], Duration::from_secs(15))?;
    let remotes = output
        .stdout
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            (fields.len() >= 3).then(|| {
                serde_json::json!({
                    "name":fields[0],
                    "url":redact_remote_url(fields[1]),
                    "direction":fields[2].trim_matches(|c| c=='(' || c==')')
                })
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"repo":repo,"remotes":remotes,"truncated":output.truncated}))
}

fn redact_remote_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return value.to_string();
    };
    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }
    url.to_string()
}

fn redact_output_urls(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|chunk| {
            let trimmed = chunk.trim_end_matches(char::is_whitespace);
            let whitespace = &chunk[trimmed.len()..];
            let (prefix, candidate, suffix) = split_url_punctuation(trimmed);
            format!(
                "{prefix}{}{suffix}{whitespace}",
                redact_remote_url(candidate)
            )
        })
        .collect()
}

fn split_url_punctuation(value: &str) -> (&str, &str, &str) {
    let prefix_len = value.len() - value.trim_start_matches(['(', '[', '<', '\'', '"']).len();
    let without_prefix = &value[prefix_len..];
    let candidate_len = without_prefix
        .trim_end_matches([')', ']', '>', ',', ';', '\'', '"'])
        .len();
    (
        &value[..prefix_len],
        &without_prefix[..candidate_len],
        &without_prefix[candidate_len..],
    )
}

fn validate_ref(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.contains('\0') {
        Err("remote/branch value is invalid".into())
    } else {
        Ok(())
    }
}

fn fetch(repo: &str, remote: Option<&str>) -> Result<serde_json::Value, String> {
    let mut args = vec!["fetch", "--prune"];
    if let Some(remote) = remote {
        validate_ref(remote)?;
        args.push(remote);
    }
    mutation(repo, "fetch", &args)
}

fn pull(
    repo: &str,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut args = vec!["pull", "--ff-only"];
    if let Some(remote) = remote {
        validate_ref(remote)?;
        args.push(remote);
    }
    if let Some(branch) = branch {
        validate_ref(branch)?;
        if remote.is_none() {
            return Err("branch requires remote for pull".into());
        }
        args.push(branch);
    }
    mutation(repo, "pull", &args)
}

fn push(
    repo: &str,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut args = vec!["push"];
    if let Some(remote) = remote {
        validate_ref(remote)?;
        args.push(remote);
    }
    if let Some(branch) = branch {
        validate_ref(branch)?;
        if remote.is_none() {
            return Err("branch requires remote for push".into());
        }
        args.push(branch);
    }
    mutation(repo, "push", &args)
}

fn mutation(repo: &str, action: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    let output = git(repo, args, Duration::from_secs(120))?;
    Ok(serde_json::json!({
        "ok":true,
        "repo":repo,
        "action":action,
        "stdout":redact_output_urls(&output.stdout),
        "stderr":redact_output_urls(&output.stderr),
        "truncated":output.truncated
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn redacts_http_remote_credentials() {
        assert_eq!(
            redact_remote_url("https://user:token@example.com/repo.git"),
            "https://example.com/repo.git"
        );
        assert_eq!(
            redact_remote_url("git@example.com:repo.git"),
            "git@example.com:repo.git"
        );
    }

    #[test]
    fn status_works_in_temporary_repo() {
        let dir = tempdir().unwrap();
        let mut init = Command::new("git");
        init.args(["init", "-q"]).current_dir(dir.path());
        require_success(run(&mut init, Duration::from_secs(5), 4096).unwrap()).unwrap();
        let value = status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(value["clean"], true);
    }
}
