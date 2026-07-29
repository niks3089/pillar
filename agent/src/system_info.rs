use std::ffi::OsStr;

use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub use sysinfo::Pid as SysPid;

/// Per-process stats from sysinfo.
pub struct ProcessStats {
    pub cpu_usage_percent: f32,
    pub memory_rss_bytes: u64,
    pub start_time_epoch_secs: u64,
}

/// Wrapper around sysinfo for collecting system-level info.
pub struct SystemInfo {
    system: System,
    networks: Networks,
    disks: Disks,
}

impl SystemInfo {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        system.refresh_memory();

        Self {
            system,
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
        }
    }

    pub fn refresh(&mut self) {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.networks.refresh(true);
        self.disks.refresh(true);
    }

    /// Refresh all processes (needed for name-based lookups).
    pub fn refresh_all_processes(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
    }

    /// Get stats for a process by PID.
    pub fn process_stats(&self, pid: Pid) -> Option<ProcessStats> {
        let p = self.system.process(pid)?;
        Some(ProcessStats {
            cpu_usage_percent: p.cpu_usage(),
            memory_rss_bytes: p.memory(),
            start_time_epoch_secs: p.start_time(),
        })
    }

    /// Find first process matching a name and return its stats.
    pub fn find_process_by_name(&self, name: &str) -> Option<ProcessStats> {
        let p = self
            .system
            .processes_by_exact_name(OsStr::new(name))
            .next()?;
        Some(ProcessStats {
            cpu_usage_percent: p.cpu_usage(),
            memory_rss_bytes: p.memory(),
            start_time_epoch_secs: p.start_time(),
        })
    }

    pub fn cpu_usage_percent(&self) -> f32 {
        self.system.global_cpu_usage()
    }

    pub fn memory_used_bytes(&self) -> u64 {
        self.system.used_memory()
    }

    pub fn memory_total_bytes(&self) -> u64 {
        self.system.total_memory()
    }

    pub fn network_rx_bytes(&self) -> u64 {
        self.networks
            .values()
            .map(|data| data.total_received())
            .sum()
    }

    pub fn network_tx_bytes(&self) -> u64 {
        self.networks
            .values()
            .map(|data| data.total_transmitted())
            .sum()
    }

    pub fn disk_used_bytes(&self) -> u64 {
        self.disks
            .list()
            .iter()
            .map(|d| d.total_space().saturating_sub(d.available_space()))
            .sum()
    }

    pub fn disk_total_bytes(&self) -> u64 {
        self.disks.list().iter().map(|d| d.total_space()).sum()
    }
}

/// Established TCP connections with the given local port, from /proc/net/tcp{,6}.
/// Returns 0 on non-Linux hosts.
pub fn established_connections(local_port: u16) -> u32 {
    let mut count = 0u32;
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines().skip(1) {
            let mut fields = line.split_whitespace();
            let (Some(local), Some(_remote), Some(state)) =
                (fields.nth(1), fields.next(), fields.next())
            else {
                continue;
            };
            let port = local
                .rsplit(':')
                .next()
                .and_then(|p| u16::from_str_radix(p, 16).ok());
            if port == Some(local_port) && state == "01" {
                count += 1;
            }
        }
    }
    count
}
