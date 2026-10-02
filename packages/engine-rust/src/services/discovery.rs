//! Finds the TCP ports this host is listening on: PowerShell on Windows, `lsof` (then `ss`) elsewhere.

use std::{collections::HashMap, time::Duration};

use serde_json::{Value, json};

use crate::utils::process::hidden_command;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// One listening port and the process that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub port: u16,
    pub address: String,
    pub pid: u32,
    pub process_name: String,
}

impl ServiceInfo {
    /// The wire shape of a discovered service.
    pub fn to_json(&self) -> Value {
        json!({ "port": self.port, "address": self.address, "pid": self.pid, "processName": self.process_name })
    }
}

/// Every listening service on this host, sorted by port. Failures yield an empty list.
pub async fn discover_services() -> Vec<ServiceInfo> {
    if cfg!(windows) { discover_windows().await } else { discover_unix().await }
}

/// Run a program and return its stdout when it exits successfully within the timeout.
async fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = hidden_command(program);
    command.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output()).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn powershell(command: &str) -> Option<String> {
    const PS: &str = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";
    run(PS, &["-NoProfile", "-NonInteractive", "-Command", command]).await
}

async fn discover_windows() -> Vec<ServiceInfo> {
    let Some(raw) = powershell(
        "Get-NetTCPConnection -State Listen | Select-Object LocalAddress,LocalPort,OwningProcess | ConvertTo-Json -Compress",
    )
    .await
    else {
        return Vec::new();
    };
    let Some(mut services) = parse_net_tcp_connections(&raw) else {
        return Vec::new();
    };
    if services.is_empty() {
        return services;
    }
    let mut pids: Vec<u32> = Vec::new();
    for service in &services {
        if !pids.contains(&service.pid) {
            pids.push(service.pid);
        }
    }
    let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let names = powershell(&format!(
        "Get-Process -Id @({list}) -ErrorAction SilentlyContinue | Select-Object Id,Name | ConvertTo-Json -Compress"
    ))
    .await
    .and_then(|raw| parse_process_names(&raw));
    for service in &mut services {
        service.process_name = names
            .as_ref()
            .and_then(|names| names.get(&service.pid).cloned())
            .unwrap_or_else(|| format!("PID {}", service.pid));
    }
    services.sort_by_key(|service| service.port);
    services
}

/// A JSON value that is either one object or an array of them, as an array.
fn as_items(raw: &str) -> Option<Vec<Value>> {
    match serde_json::from_str::<Value>(raw.trim()).ok()? {
        Value::Array(items) => Some(items),
        other => Some(vec![other]),
    }
}

/// Parse `Get-NetTCPConnection ... | ConvertTo-Json`: only wildcard and loopback addresses count, and the
/// first listener of a port wins. `None` when the output is not JSON.
pub fn parse_net_tcp_connections(raw: &str) -> Option<Vec<ServiceInfo>> {
    let mut services: Vec<ServiceInfo> = Vec::new();
    for item in as_items(raw)? {
        let address = item.get("LocalAddress").and_then(Value::as_str).unwrap_or("");
        if !matches!(address, "0.0.0.0" | "127.0.0.1" | "::" | "::1") {
            continue;
        }
        let Some(port) = item.get("LocalPort").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()) else {
            continue;
        };
        if services.iter().any(|service| service.port == port) {
            continue;
        }
        let pid = item.get("OwningProcess").and_then(Value::as_u64).unwrap_or(0) as u32;
        services.push(ServiceInfo { port, address: address.to_string(), pid, process_name: String::new() });
    }
    Some(services)
}

/// Parse `Get-Process ... | Select-Object Id,Name | ConvertTo-Json` into pid -> name.
pub fn parse_process_names(raw: &str) -> Option<HashMap<u32, String>> {
    let mut names = HashMap::new();
    for item in as_items(raw)? {
        if let (Some(id), Some(name)) =
            (item.get("Id").and_then(Value::as_u64), item.get("Name").and_then(Value::as_str))
        {
            names.insert(id as u32, name.to_string());
        }
    }
    Some(names)
}

async fn discover_unix() -> Vec<ServiceInfo> {
    if let Some(raw) = run("lsof", &["-iTCP", "-sTCP:LISTEN", "-n", "-P"]).await {
        return parse_lsof_output(&raw);
    }
    match run("ss", &["-tlnp"]).await {
        Some(raw) => parse_ss_output(&raw),
        None => Vec::new(),
    }
}

/// The trailing `:port` of a field.
fn trailing_port(field: &str) -> Option<u16> {
    let (_, digits) = field.rsplit_once(':')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn collect(entries: Vec<(u16, u32, String)>) -> Vec<ServiceInfo> {
    let mut services: Vec<ServiceInfo> = Vec::new();
    for (port, pid, process_name) in entries {
        if !services.iter().any(|service| service.port == port) {
            services.push(ServiceInfo { port, address: "127.0.0.1".to_string(), pid, process_name });
        }
    }
    services.sort_by_key(|service| service.port);
    services
}

/// Parse `lsof -iTCP -sTCP:LISTEN -n -P`: command, pid and the `NAME` column (`*:8080`, `127.0.0.1:8080`).
pub fn parse_lsof_output(raw: &str) -> Vec<ServiceInfo> {
    let mut entries = Vec::new();
    for line in raw.trim().lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            continue;
        }
        let Some(port) = trailing_port(parts[8]) else { continue };
        entries.push((port, parts[1].parse().unwrap_or(0), parts[0].to_string()));
    }
    collect(entries)
}

/// Parse `ss -tlnp`: the local `addr:port` column and the `users:(("name",pid=N,fd=M))` column.
pub fn parse_ss_output(raw: &str) -> Vec<ServiceInfo> {
    let mut entries = Vec::new();
    for line in raw.trim().lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        let Some(port) = trailing_port(parts[3]) else { continue };
        let process = parts.get(5).copied().unwrap_or("");
        let pid = process
            .split_once("pid=")
            .map(|(_, rest)| rest.chars().take_while(char::is_ascii_digit).collect::<String>())
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0);
        let name = process
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(name, _)| name.to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("PID {pid}"));
        entries.push((port, pid, name));
    }
    collect(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_connections_filter_addresses_and_dedupe_ports() {
        let raw = r#"[{"LocalAddress":"0.0.0.0","LocalPort":135,"OwningProcess":1100},
            {"LocalAddress":"192.168.1.5","LocalPort":139,"OwningProcess":4},
            {"LocalAddress":"::","LocalPort":135,"OwningProcess":1100},
            {"LocalAddress":"127.0.0.1","LocalPort":3000,"OwningProcess":4242}]"#;
        let services = parse_net_tcp_connections(raw).unwrap();
        assert_eq!(services.len(), 2);
        assert_eq!((services[0].port, services[0].address.as_str(), services[0].pid), (135, "0.0.0.0", 1100));
        assert_eq!(services[1].port, 3000);
    }

    #[test]
    fn windows_single_object_and_garbage() {
        let one = r#"{"LocalAddress":"::1","LocalPort":8080,"OwningProcess":7}"#;
        assert_eq!(parse_net_tcp_connections(one).unwrap().len(), 1);
        assert!(parse_net_tcp_connections("").is_none());
        let names = parse_process_names(r#"[{"Id":7,"Name":"node"},{"Id":8,"Name":"svchost"}]"#).unwrap();
        assert_eq!(names.get(&7).map(String::as_str), Some("node"));
        let single = parse_process_names(r#"{"Id":9,"Name":"deno"}"#).unwrap();
        assert_eq!(single.get(&9).map(String::as_str), Some("deno"));
    }

    #[test]
    fn lsof_output_is_parsed_deduped_and_sorted() {
        let raw = "COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n\
node    4242 me   23u  IPv4 0x1234      0t0  TCP *:8080 (LISTEN)\n\
node    4242 me   24u  IPv6 0x1235      0t0  TCP *:8080 (LISTEN)\n\
postgres 900 me    5u  IPv4 0x1236      0t0  TCP 127.0.0.1:5432 (LISTEN)\n\
short line\n";
        let services = parse_lsof_output(raw);
        assert_eq!(services.len(), 2);
        assert_eq!((services[0].port, services[0].pid, services[0].process_name.as_str()), (5432, 900, "postgres"));
        assert_eq!((services[1].port, services[1].address.as_str()), (8080, "127.0.0.1"));
    }

    #[test]
    fn ss_output_is_parsed() {
        let raw = "State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
LISTEN 0      511          0.0.0.0:3000      0.0.0.0:*     users:((\"node\",pid=321,fd=19))\n\
LISTEN 0      128             [::]:22           [::]:*\n\
LISTEN 0      128        127.0.0.1:631       0.0.0.0:*\n";
        let services = parse_ss_output(raw);
        assert_eq!(services.len(), 3);
        assert_eq!((services[0].port, services[0].process_name.as_str(), services[0].pid), (22, "PID 0", 0));
        assert_eq!((services[2].port, services[2].process_name.as_str(), services[2].pid), (3000, "node", 321));
    }
}
