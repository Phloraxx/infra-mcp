use infra_protocol::{OperationResult, SystemAction, SystemRequest};
use serde::Serialize;
use sysinfo::{Disks, Networks, System};

#[derive(Debug, Serialize)]
struct SystemSummary {
    hostname: Option<String>,
    os_name: Option<String>,
    os_version: Option<String>,
    kernel_version: Option<String>,
    architecture: &'static str,
    uptime_seconds: u64,
    cpu_count: usize,
    cpu_brand: Option<String>,
    total_memory: u64,
    used_memory: u64,
    total_swap: u64,
    used_swap: u64,
    load_one: f64,
    load_five: f64,
    load_fifteen: f64,
}

#[derive(Debug, Serialize)]
struct DiskView {
    name: String,
    file_system: String,
    mount_point: String,
    total_space: u64,
    available_space: u64,
    removable: bool,
}

#[derive(Debug, Serialize)]
struct NetworkView {
    name: String,
    received: u64,
    transmitted: u64,
    total_received: u64,
    total_transmitted: u64,
    mac_address: String,
    addresses: Vec<String>,
}

pub fn handle(request: SystemRequest) -> OperationResult {
    let value = match request.action {
        SystemAction::Summary => summary().and_then(json),
        SystemAction::Disks => disks().and_then(json),
        SystemAction::Networks => networks().and_then(json),
    };
    match value {
        Ok(body) => OperationResult::Ok { body },
        Err(error) => OperationResult::Failed { error },
    }
}

fn json(value: impl Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn summary() -> Result<SystemSummary, String> {
    let mut system = System::new_all();
    system.refresh_all();
    let load = System::load_average();
    Ok(SystemSummary {
        hostname: System::host_name(),
        os_name: System::name(),
        os_version: System::os_version(),
        kernel_version: Some(System::kernel_long_version()),
        architecture: std::env::consts::ARCH,
        uptime_seconds: System::uptime(),
        cpu_count: system.cpus().len(),
        cpu_brand: system.cpus().first().map(|cpu| cpu.brand().to_string()),
        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
        load_one: load.one,
        load_five: load.five,
        load_fifteen: load.fifteen,
    })
}

fn disks() -> Result<Vec<DiskView>, String> {
    let disks = Disks::new_with_refreshed_list();
    Ok(disks
        .list()
        .iter()
        .map(|disk| DiskView {
            name: disk.name().to_string_lossy().into_owned(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            mount_point: disk.mount_point().to_string_lossy().into_owned(),
            total_space: disk.total_space(),
            available_space: disk.available_space(),
            removable: disk.is_removable(),
        })
        .collect())
}

fn networks() -> Result<Vec<NetworkView>, String> {
    let networks = Networks::new_with_refreshed_list();
    Ok(networks
        .iter()
        .map(|(name, data)| NetworkView {
            name: name.clone(),
            received: data.received(),
            transmitted: data.transmitted(),
            total_received: data.total_received(),
            total_transmitted: data.total_transmitted(),
            mac_address: data.mac_address().to_string(),
            addresses: data.ip_networks().iter().map(ToString::to_string).collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_has_memory_and_cpu() {
        let summary = summary().unwrap();
        assert!(summary.cpu_count > 0);
        assert!(summary.total_memory > 0);
    }

    #[test]
    fn networks_and_disks_are_serializable() {
        serde_json::to_value(disks().unwrap()).unwrap();
        serde_json::to_value(networks().unwrap()).unwrap();
    }
}
