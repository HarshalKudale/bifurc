//! Port utilities: whether a local TCP port is taken, and killing the process that holds it.

use std::net::{Ipv4Addr, TcpListener};

use crate::{
    protocol::{EngineError, Result},
    utils::process::hidden_command,
};

/// Whether a local TCP port is taken and, if so, the process that owns it (when it can be found).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortInfo {
    pub in_use: bool,
    pub pid: Option<u32>,
}

impl PortInfo {
    pub fn to_json(self) -> serde_json::Value {
        let mut out = serde_json::json!({ "inUse": self.in_use });
        if let Some(pid) = self.pid {
            out["pid"] = pid.into();
        }
        out
    }
}

/// Check whether `port` is in use on the loopback interface. Any failure to bind counts as "in use".
pub async fn check_port_in_use(port: i64) -> Result<PortInfo> {
    let port = u16::try_from(port)
        .map_err(|_| EngineError::engine(format!("options.port should be >= 0 and < 65536. Received {port}.")))?;
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(PortInfo { in_use: false, pid: None })
        }
        Err(_) => Ok(PortInfo { in_use: true, pid: find_pid_on_port(port).await }),
    }
}

/// Kill whatever process is listening on `port`. `false` when no owner was found or the kill failed.
pub async fn kill_process_on_port(port: i64) -> bool {
    let Ok(port) = u16::try_from(port) else { return false };
    let Some(pid) = find_pid_on_port(port).await else { return false };
    kill_pid(pid).await
}

#[cfg(windows)]
async fn kill_pid(pid: u32) -> bool {
    hidden_command("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
async fn kill_pid(pid: u32) -> bool {
    // SAFETY: `kill` takes plain integers and has no memory-safety preconditions.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) == 0 }
}

#[cfg(windows)]
async fn find_pid_on_port(port: u16) -> Option<u32> {
    let out = hidden_command("netstat").args(["-ano", "-p", "TCP"]).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_netstat_pid(&String::from_utf8_lossy(&out.stdout), port)
}

#[cfg(not(windows))]
async fn find_pid_on_port(port: u16) -> Option<u32> {
    let out = hidden_command("lsof").args(["-t", "-i", &format!(":{port}")]).output().await.ok()?;
    parse_lsof_pid(&String::from_utf8_lossy(&out.stdout))
}

/// The PID of the first `LISTENING` line of `netstat -ano -p TCP` output on a wildcard or loopback address at
/// `port`. Lines look like `TCP    0.0.0.0:3000    0.0.0.0:0    LISTENING    1234`.
pub fn parse_netstat_pid(stdout: &str, port: u16) -> Option<u32> {
    let pattern = format!(r"(?:0\.0\.0\.0|127\.0\.0\.1|\[::\]):{port}\s+\S+\s+LISTENING\s+(\d+)");
    let re = regress::Regex::with_flags(&pattern, "i").ok()?;
    let found = re.find(stdout)?;
    let range = found.group(1)?;
    stdout[range].parse().ok()
}

/// The PID on the first line of `lsof -t -i :PORT` output (`parseInt` of the line; zero and junk are none).
pub fn parse_lsof_pid(stdout: &str) -> Option<u32> {
    let line = stdout.trim().split('\n').next()?;
    let digits: String = line.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u32>().ok().filter(|pid| *pid != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETSTAT: &str = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1100\r\n  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4242\r\n  TCP    127.0.0.1:5173         0.0.0.0:0              LISTENING       777\r\n  TCP    [::]:8080              [::]:0                 LISTENING       999\r\n  TCP    192.168.1.5:3000       10.0.0.1:50000         ESTABLISHED     55\r\n";

    #[test]
    fn netstat_finds_listening_owner() {
        assert_eq!(parse_netstat_pid(NETSTAT, 3000), Some(4242));
        assert_eq!(parse_netstat_pid(NETSTAT, 5173), Some(777));
        assert_eq!(parse_netstat_pid(NETSTAT, 8080), Some(999));
        assert_eq!(parse_netstat_pid(NETSTAT, 135), Some(1100));
    }

    #[test]
    fn netstat_ignores_other_ports_and_states() {
        assert_eq!(parse_netstat_pid(NETSTAT, 4000), None);
        assert_eq!(parse_netstat_pid(NETSTAT, 50000), None);
        assert_eq!(parse_netstat_pid("", 3000), None);
        assert_eq!(parse_netstat_pid("  tcp    0.0.0.0:3000   0.0.0.0:0   listening   12\r\n", 3000), Some(12));
    }

    #[test]
    fn lsof_takes_the_first_pid() {
        assert_eq!(parse_lsof_pid("1234\n5678\n"), Some(1234));
        assert_eq!(parse_lsof_pid("  42  \n"), Some(42));
        assert_eq!(parse_lsof_pid(""), None);
        assert_eq!(parse_lsof_pid("0\n"), None);
        assert_eq!(parse_lsof_pid("lsof: not found\n"), None);
    }

    #[tokio::test]
    async fn bound_port_is_in_use_and_free_port_is_not() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = i64::from(listener.local_addr().unwrap().port());
        let info = check_port_in_use(port).await.unwrap();
        assert!(info.in_use);
        if let Some(pid) = info.pid {
            assert_eq!(pid, std::process::id());
        }
        drop(listener);
        assert_eq!(check_port_in_use(port).await.unwrap(), PortInfo { in_use: false, pid: None });
    }

    #[tokio::test]
    async fn out_of_range_port_is_an_error() {
        assert!(check_port_in_use(70000).await.is_err());
        assert!(!kill_process_on_port(70000).await);
    }

    #[test]
    fn port_info_omits_absent_pid() {
        assert_eq!(PortInfo { in_use: false, pid: None }.to_json(), serde_json::json!({ "inUse": false }));
        assert_eq!(PortInfo { in_use: true, pid: Some(3) }.to_json(), serde_json::json!({ "inUse": true, "pid": 3 }));
    }
}
