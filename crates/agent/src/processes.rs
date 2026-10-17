use infra_protocol::{OperationResult, ProcessAction, ProcessRequest};
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

const MAX_PROCESS_RESULTS: usize = 500;

#[derive(Debug, Serialize)]
struct ProcessView {
    pid: u32,
    parent_pid: Option<u32>,
    name: String,
    command: String,
    cpu_percent: f32,
    memory_bytes: u64,
}

pub fn handle(request: ProcessRequest) -> OperationResult {
    match request.action {
        ProcessAction::List { query } => list(query),
        ProcessAction::Terminate { pid, force } => terminate(pid, force),
    }
}

fn refreshed_system() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_cmd(UpdateKind::Always),
    );
    system
}

fn list(query: Option<String>) -> OperationResult {
    let system = refreshed_system();
    let query = query.map(|value| value.to_lowercase());
    let mut processes = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().into_owned();
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(query) = &query
                && !format!("{name} {command}").to_lowercase().contains(query)
            {
                return None;
            }
            Some(ProcessView {
                pid: pid.as_u32(),
                parent_pid: process.parent().map(Pid::as_u32),
                name,
                command,
                cpu_percent: process.cpu_usage(),
                memory_bytes: process.memory(),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| std::cmp::Reverse(process.pid));
    let total = processes.len();
    processes.truncate(MAX_PROCESS_RESULTS);
    OperationResult::Ok {
        body: serde_json::json!({
            "processes": processes,
            "count": total,
            "truncated": total > MAX_PROCESS_RESULTS,
        }),
    }
}

pub fn terminate_process_tree(pid: u32, force: bool) -> Result<(), String> {
    if pid == std::process::id() {
        return Err("refusing to terminate the Infra agent process itself".into());
    }
    let system = refreshed_system();
    let target = Pid::from_u32(pid);
    if system.process(target).is_none() {
        return Err("process not found".into());
    }

    let mut descendants = Vec::new();
    collect_descendants(&system, target, &mut descendants);
    descendants.reverse();
    descendants.push(target);
    let signal = if force { Signal::Kill } else { Signal::Term };
    let mut failed = Vec::new();
    for candidate in descendants {
        if let Some(process) = system.process(candidate) {
            let sent = process.kill_with(signal).unwrap_or_else(|| process.kill());
            if !sent {
                failed.push(candidate.as_u32());
            }
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("failed to signal process(es): {failed:?}"))
    }
}

fn collect_descendants(system: &System, parent: Pid, out: &mut Vec<Pid>) {
    for (pid, process) in system.processes() {
        if process.parent() == Some(parent) {
            out.push(*pid);
            collect_descendants(system, *pid, out);
        }
    }
}

fn terminate(pid: u32, force: bool) -> OperationResult {
    match terminate_process_tree(pid, force) {
        Ok(()) => OperationResult::Ok {
            body: serde_json::json!({"ok": true, "pid": pid, "force": force}),
        },
        Err(error) => OperationResult::Failed { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_returns_current_process() {
        let OperationResult::Ok { body } = list(Some("infra".into())) else {
            panic!("expected process list");
        };
        assert!(body["count"].as_u64().is_some());
    }
}
