//! Passive host evidence only. No commands, pipes, engine probes or config files.
use crate::{
    devcheck::{probe, Probe},
    snapshot::Progress,
};
use std::path::PathBuf;
#[cfg(windows)]
mod windows;

const CLIENTS: &[&str] = &["wsl.exe", "docker.exe", "podman.exe", "nerdctl.exe"];
const OVERRIDES: &[&str] = &[
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "DOCKER_CONFIG",
    "DOCKER_TLS_VERIFY",
    "DOCKER_CERT_PATH",
    "CONTAINER_HOST",
    "CONTAINER_CONNECTION",
];
const SERVICES: &[&str] = &[
    "WslService",
    "LxssManager",
    "vmcompute",
    "hns",
    "com.docker.service",
    "docker",
];
pub const LIMITATIONS: &str = "Passive evidence only: CLI files and running services do not prove WSL or a container engine works. Distributions, optional-feature enablement, WSL versions, active contexts, daemon health, Linux/Windows container mode, nested virtualization, permissions and project compatibility remain unverified. No command, engine, distribution, network/pipe connection, configuration file read or system change is requested. Service names are conventional hints; missing services do not establish absence. CPU hints can be masked by a hypervisor; false does not prove firmware virtualization is disabled.";
#[derive(Clone, Debug)]
pub struct Client {
    pub name: &'static str,
    pub candidates: Vec<PathBuf>,
    pub partial: bool,
}
#[derive(Clone, Debug)]
pub struct Observation {
    pub name: &'static str,
    pub evidence: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub started: String,
    pub finished: String,
    pub clients: Vec<Client>,
    pub host: Vec<Observation>,
    pub routing_overrides: Vec<&'static str>,
    pub notes: Vec<String>,
    pub partial: bool,
    pub cancelled: bool,
}
impl Report {
    pub fn text(&self) -> String {
        let mut text = format!("WSL / container host evidence\nStarted: {}\nFinished: {}\nPartial: {} · Cancelled: {}\n{}\n", self.started, self.finished, self.partial, self.cancelled, LIMITATIONS);
        for client in &self.clients {
            text.push_str(&format!(
                "\n{}: {} candidate(s); partial: {}\n",
                client.name,
                client.candidates.len(),
                client.partial
            ));
            for path in &client.candidates {
                text.push_str(&format!("  {}\n", path.display()));
            }
        }
        for item in &self.host {
            text.push_str(&format!("\n{}: {}\n", item.name, item.evidence));
        }
        text.push_str("\nRouting/config override names present (values omitted):\n");
        if self.routing_overrides.is_empty() {
            text.push_str("None of the checked names observed; default/config-file routing remains unverified.\n");
        }
        for name in &self.routing_overrides {
            text.push_str(name);
            text.push('\n');
        }
        for note in &self.notes {
            text.push_str(note);
            text.push('\n');
        }
        text
    }
}
fn discover(path: &str, system: Option<PathBuf>, progress: &Progress) -> (Vec<Client>, bool) {
    let mut directories = Vec::new();
    let mut partial = false;
    if let Some(system) = system {
        directories.push(system);
    }
    for (index, entry) in path.split(';').enumerate() {
        if progress.cancelled() {
            partial = true;
            break;
        }
        if index >= 256 {
            partial = true;
            break;
        }
        let entry = entry.trim();
        if entry.is_empty()
            || entry.contains(['%', '"', '\0'])
            || entry.encode_utf16().count() > 4096
        {
            partial = true;
            continue;
        }
        let dir = PathBuf::from(entry);
        match probe(&dir) {
            Probe::Dir => {
                if !directories.iter().any(|d: &PathBuf| {
                    d.to_string_lossy()
                        .eq_ignore_ascii_case(&dir.to_string_lossy())
                }) {
                    directories.push(dir);
                }
            }
            _ => partial = true,
        }
    }
    let mut clients = Vec::new();
    for &name in CLIENTS {
        let mut client = Client {
            name,
            candidates: Vec::new(),
            partial,
        };
        for dir in &directories {
            if progress.cancelled() {
                client.partial = true;
                break;
            }
            progress.tick();
            match probe(&dir.join(name)) {
                Probe::File => {
                    if client.candidates.len() == 32 {
                        client.partial = true;
                        break;
                    }
                    client.candidates.push(dir.join(name));
                }
                Probe::Missing => {}
                _ => client.partial = true,
            }
        }
        partial |= client.partial;
        clients.push(client);
    }
    (clients, partial)
}
pub fn inspect(progress: &Progress) -> Report {
    progress.set_phase("Passive WSL/container inspection");
    let mut report = Report {
        started: crate::util::now_string(),
        finished: String::new(),
        clients: Vec::new(),
        host: Vec::new(),
        routing_overrides: Vec::new(),
        notes: Vec::new(),
        partial: false,
        cancelled: false,
    };
    let path = match std::env::var("PATH") {
        Ok(path) if path.len() <= 65536 => path,
        _ => {
            report.partial = true;
            report
                .notes
                .push("PATH unavailable or exceeds 64 KiB; PATH discovery skipped.".into());
            String::new()
        }
    };
    #[cfg(windows)]
    let system = match windows::system_directory() {
        Ok(dir) => Some(dir),
        Err(error) => {
            report.partial = true;
            report.notes.push(error);
            None
        }
    };
    #[cfg(not(windows))]
    let system = None;
    let (clients, partial) = discover(&path, system, progress);
    report.clients = clients;
    report.partial |= partial;
    for &name in OVERRIDES {
        if progress.cancelled() {
            break;
        }
        if std::env::var_os(name).is_some() {
            report.routing_overrides.push(name);
        }
    }
    #[cfg(windows)]
    windows::host(&mut report, progress);
    #[cfg(not(windows))]
    {
        report.partial = true;
        report
            .notes
            .push("Windows service and processor evidence unsupported on this OS.".into());
    }
    report.cancelled = progress.cancelled();
    report.partial |= report.cancelled;
    report.finished = crate::util::now_string();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_regular_fixed_clients_no_execution() {
        let dir = std::env::temp_dir().join(format!(
            "setupscope-container-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("docker.exe"), b"deliberately not executable").unwrap();
        std::fs::create_dir(dir.join("podman.exe")).unwrap();
        let (clients, partial) = discover(dir.to_str().unwrap(), None, &Progress::default());
        assert_eq!(clients[1].candidates, vec![dir.join("docker.exe")]);
        assert!(clients[2].candidates.is_empty());
        assert!(partial);
        assert!(clients[0].candidates.is_empty());
        let (dedup, _) = discover(
            &format!("{};{}", dir.display(), dir.display()),
            None,
            &Progress::default(),
        );
        assert_eq!(dedup[1].candidates.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unsafe_and_excessive_inputs_partial() {
        let (_, partial) = discover(
            ";%UNKNOWN%;relative;\"C:\\quoted\"",
            None,
            &Progress::default(),
        );
        assert!(partial);
        let (_, partial) = discover(&"missing;".repeat(257), None, &Progress::default());
        assert!(partial);
        let (_, partial) = discover(&"x".repeat(4097), None, &Progress::default());
        assert!(partial);
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (clients, partial) = discover("", None, &progress);
        assert!(partial);
        assert!(clients.iter().all(|c| c.candidates.is_empty()));
    }
}
