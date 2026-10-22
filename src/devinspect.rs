//! Read-only local endpoint and selected-file ownership observations.
use crate::snapshot::Progress;
use anyhow::{ensure, Result};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddrV6},
    path::{Path, PathBuf},
};
#[cfg(windows)]
mod windows;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transport {
    Tcp,
    Udp,
}
impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP listener",
            Self::Udp => "UDP bound endpoint",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    pub transport: Transport,
    pub address: IpAddr,
    pub scope_id: u32,
    pub port: u16,
    pub pid: u32,
}
impl Endpoint {
    pub fn local_address(&self) -> String {
        match self.address {
            IpAddr::V4(ip) => format!("{ip}:{}", self.port),
            IpAddr::V6(ip) => SocketAddrV6::new(ip, self.port, 0, self.scope_id).to_string(),
        }
    }
    pub fn binding(&self) -> &'static str {
        if self.address.is_loopback() {
            "Loopback bind; reachability untested"
        } else if self.address.is_unspecified() {
            "Wildcard bind; exposure unknown"
        } else {
            "Specific local address; exposure unknown"
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    PidOnly,
    StartMatched,
    StartMismatch,
    Unavailable,
    Skipped,
}
impl Identity {
    pub fn label(self) -> &'static str {
        match self {
            Self::PidOnly => "Current PID metadata; endpoint-time identity unverified",
            Self::StartMatched => "Reported process start time matched",
            Self::StartMismatch => "PID/start-time mismatch; image withheld",
            Self::Unavailable => "Process identity unavailable",
            Self::Skipped => "Process metadata not inspected",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProcessEvidence {
    pub pid: u32,
    pub creation: Option<u64>,
    pub image: Option<String>,
    pub identity: Identity,
    pub detail: String,
}
impl ProcessEvidence {
    pub(crate) fn skipped(pid: u32, detail: &str) -> Self {
        Self {
            pid,
            creation: None,
            image: None,
            identity: Identity::Skipped,
            detail: detail.into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Source {
    pub name: &'static str,
    pub rows: usize,
    pub error: Option<String>,
}
pub struct PortReport {
    pub started: String,
    pub finished: String,
    pub endpoints: Vec<Endpoint>,
    pub processes: BTreeMap<u32, ProcessEvidence>,
    pub sources: Vec<Source>,
    pub partial: bool,
    pub cancelled: bool,
}
#[derive(Clone, Debug)]
pub struct LockOwner {
    pub pid: u32,
    pub reported_start: u64,
    pub name: String,
    pub service: String,
    pub process: ProcessEvidence,
}
pub struct LockReport {
    pub path: PathBuf,
    pub started: String,
    pub finished: String,
    pub owners: Vec<LockOwner>,
    pub reboot_reasons: u32,
    pub notes: Vec<String>,
    pub partial: bool,
    pub cancelled: bool,
}
pub fn inspect_ports(include_udp: bool, progress: &Progress) -> Result<PortReport> {
    #[cfg(windows)]
    {
        windows::ports(include_udp, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (include_udp, progress);
        anyhow::bail!("Local endpoint inspection requires Windows")
    }
}
pub fn inspect_file(path: &Path, progress: &Progress) -> Result<LockReport> {
    #[cfg(windows)]
    {
        windows::locks(path, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, progress);
        anyhow::bail!("File ownership inspection requires Windows Restart Manager")
    }
}
#[derive(Clone, Copy, Default)]
pub struct Filter {
    pub port: Option<u16>,
    pub pid: Option<u32>,
}
impl Filter {
    pub fn parse(port: &str, pid: &str) -> Result<Self> {
        fn number<T: std::str::FromStr>(value: &str) -> Result<Option<T>> {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                ensure!(
                    value.trim().bytes().all(|b| b.is_ascii_digit()),
                    "Filters require decimal digits only"
                );
                Ok(Some(value.trim().parse().map_err(|_| {
                    anyhow::anyhow!("Filter is outside its numeric range")
                })?))
            }
        }
        Ok(Self {
            port: number(port)?,
            pid: number(pid)?,
        })
    }
    fn matches(self, endpoint: &Endpoint) -> bool {
        self.port.is_none_or(|p| p == endpoint.port) && self.pid.is_none_or(|p| p == endpoint.pid)
    }
}
pub struct Page {
    pub text: String,
    pub pages: usize,
    pub count: usize,
    pub page: usize,
}
fn append(output: &mut String, text: &str) {
    const LIMIT: usize = 128 * 1024;
    const NOTICE:&str="\n[Page truncated at 128 KiB; use one row per page or inspect the selected path separately.]\n";
    if output.ends_with(NOTICE) {
        return;
    }
    let room = LIMIT.saturating_sub(output.len() + NOTICE.len());
    if text.len() <= room {
        output.push_str(text);
    } else {
        let mut n = room.min(text.len());
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        output.push_str(&text[..n]);
        output.push_str(NOTICE);
    }
}
fn process_text(process: &ProcessEvidence) -> String {
    format!(
        "{}; creation FILETIME: {}\nImage: {}\n{}",
        process.identity.label(),
        process
            .creation
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".into()),
        process.image.as_deref().unwrap_or("unknown / withheld"),
        process.detail
    )
}
impl PortReport {
    pub fn page(&self, filter: Filter, page: usize, one_row: bool) -> Page {
        let count = self.endpoints.iter().filter(|e| filter.matches(e)).count();
        let rows = if one_row { 1 } else { 50 };
        let pages = count.max(1).div_ceil(rows);
        let page = page.min(pages - 1);
        let mut text=format!("Local endpoint observation: {} → {}\n{} · {} matching rows · page {}/{}\nTables are sequential snapshots, not an atomic reservation or current reachability test. UDP rows are bound endpoints, not TCP listeners. PID reuse between table/metadata reads cannot be excluded. No DNS, connections, firewall changes or process control.\n",self.started,self.finished,if self.partial{"PARTIAL / UNKNOWN COVERAGE"}else{"Tables queried within configured bounds"},count,page+1,pages);
        for source in &self.sources {
            append(
                &mut text,
                &format!(
                    "{}: {} rows; {}\n",
                    source.name,
                    source.rows,
                    source
                        .error
                        .as_deref()
                        .unwrap_or("query succeeded at observation time")
                ),
            );
        }
        if self.cancelled {
            append(
                &mut text,
                "Cancelled: unqueried sources/metadata remain unknown.\n",
            );
        }
        if count == 0 {
            append(&mut text,"No matching rows observed. This does not prove a port is free, reachable or free of excluded/system activity.\n");
        }
        for endpoint in self
            .endpoints
            .iter()
            .filter(|e| filter.matches(e))
            .skip(page * rows)
            .take(rows)
        {
            append(
                &mut text,
                &format!(
                    "\n{} · {} · PID {}\n{}\n",
                    endpoint.transport.label(),
                    endpoint.local_address(),
                    endpoint.pid,
                    endpoint.binding()
                ),
            );
            if let Some(process) = self.processes.get(&endpoint.pid) {
                append(&mut text, &process_text(process));
                append(&mut text, "\n");
            }
        }
        Page {
            text,
            pages,
            count,
            page,
        }
    }
}
impl LockReport {
    pub fn page(&self, page: usize, one_row: bool) -> Page {
        let count = self.owners.len();
        let rows = if one_row { 1 } else { 50 };
        let pages = count.max(1).div_ceil(rows);
        let page = page.min(pages - 1);
        let mut text=format!("Selected-file Restart Manager observation: {} → {}\n{} · {} reported owners · page {}/{}\n",self.started,self.finished,if self.partial{"PARTIAL / UNKNOWN COVERAGE"}else{"Query succeeded within configured bounds"},count,page+1,pages);
        append(
            &mut text,
            &format!("Selected file: {}\n", self.path.display()),
        );
        append(&mut text,"Restart Manager is not a complete handle/lock inventory. Zero reported owners does not prove the file is unlocked. No shutdown, restart, unlock, deletion or process termination is requested. Paths/processes can change after observation.\n");
        append(
            &mut text,
            &format!(
                "Reported reboot-reason mask: 0x{:X} (no reboot action requested)\n",
                self.reboot_reasons
            ),
        );
        for note in &self.notes {
            append(&mut text, &format!("Note: {note}\n"));
        }
        if self.cancelled {
            append(
                &mut text,
                "Cancelled: remaining process metadata was not inspected.\n",
            );
        }
        for owner in self.owners.iter().skip(page * rows).take(rows) {
            append(
                &mut text,
                &format!(
                    "\nReported PID {} · {} · service: {}\nReported creation FILETIME: {}\n",
                    owner.pid,
                    owner.name,
                    if owner.service.is_empty() {
                        "none reported"
                    } else {
                        &owner.service
                    },
                    owner.reported_start
                ),
            );
            append(&mut text, &process_text(&owner.process));
            append(&mut text, "\n");
        }
        Page {
            text,
            pages,
            count,
            page,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_are_bounded_decimal_values() {
        assert!(Filter::parse("65535", "4294967295").is_ok());
        for (port, pid) in [
            ("65536", ""),
            ("-1", ""),
            ("", "4294967296"),
            ("80;echo", ""),
            ("+80", ""),
        ] {
            assert!(Filter::parse(port, pid).is_err());
        }
    }
    #[test]
    fn address_scopes_and_zero_results_keep_unknowns_explicit() {
        let endpoint = Endpoint {
            transport: Transport::Udp,
            address: "fe80::1".parse().unwrap(),
            scope_id: 7,
            port: 5353,
            pid: 1,
        };
        assert_eq!(endpoint.local_address(), "[fe80::1%7]:5353");
        let report = PortReport {
            started: "fixture".into(),
            finished: "fixture".into(),
            endpoints: vec![endpoint],
            processes: BTreeMap::new(),
            sources: vec![Source {
                name: "fixture",
                rows: 0,
                error: Some("access denied".into()),
            }],
            partial: true,
            cancelled: false,
        };
        let page = report.page(
            Filter {
                port: Some(80),
                pid: None,
            },
            0,
            false,
        );
        assert!(
            page.text.contains("access denied")
                && page.text.contains("does not prove a port is free")
        );
    }
    #[test]
    fn paging_filtering_and_unicode_caps_preserve_report_bounds() {
        let endpoints = (0..101)
            .map(|i| Endpoint {
                transport: Transport::Tcp,
                address: "127.0.0.1".parse().unwrap(),
                scope_id: 0,
                port: 8000 + i,
                pid: 2,
            })
            .collect();
        let report = PortReport {
            started: "fixture".into(),
            finished: "fixture".into(),
            endpoints,
            processes: BTreeMap::new(),
            sources: vec![],
            partial: false,
            cancelled: false,
        };
        assert_eq!(report.page(Filter::default(), 2, false).pages, 3);
        assert_eq!(
            report
                .page(
                    Filter {
                        port: Some(8080),
                        pid: None
                    },
                    0,
                    false
                )
                .count,
            1
        );
        let mut text = String::new();
        append(&mut text, &"🦀".repeat(100_000));
        assert!(text.len() <= 128 * 1024 && text.contains("truncated"));
    }
}
