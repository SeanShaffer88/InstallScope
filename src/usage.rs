//! How programs are actually used: when each was last opened (from Windows'
//! own records: UserAssist and Prefetch), how much memory and CPU it uses
//! (sampled now and then while SetupSweep or background monitoring runs), and
//! how much it slows down startup. Everything stays on this PC.

use crate::platform::Registry;
use crate::programs::InstalledProgram;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// ROT13, which UserAssist uses for its value names.
pub fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
            _ => c,
        })
        .collect()
}

/// FILETIME (100 ns since 1601) → Unix seconds.
pub fn filetime_to_unix(ft: u64) -> Option<i64> {
    if ft == 0 {
        return None;
    }
    Some((ft / 10_000_000) as i64 - 11_644_473_600)
}

/// Known folder GUIDs UserAssist uses in place of paths.
fn known_folder(guid: &str) -> Option<String> {
    let env = |k: &str| std::env::var(k).ok();
    match guid.to_ascii_uppercase().as_str() {
        "{6D809377-6AF0-444B-8957-A3773F02200E}" => {
            env("ProgramW6432").or_else(|| env("ProgramFiles"))
        }
        "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}" => env("ProgramFiles(x86)"),
        "{F38BF404-1D43-42F2-9305-67DE0B28FC23}" => env("windir"),
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}" => env("windir").map(|w| format!("{w}\\System32")),
        "{D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}" => env("windir").map(|w| format!("{w}\\SysWOW64")),
        "{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}" => env("LOCALAPPDATA"),
        "{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}" => env("APPDATA"),
        "{62AB5D82-FDC1-4DC3-A9DD-070D1D495D97}" => env("ProgramData"),
        _ => None,
    }
}

/// One program file's record in UserAssist: path, run count, last run.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub path: String,
    pub runs: u32,
    pub last: Option<i64>,
}

/// Decode one UserAssist value (name already ROT13-decoded).
pub fn decode_userassist(name: &str, data: &[u8]) -> Option<Launch> {
    if data.len() < 68 {
        return None;
    }
    let path = match name.strip_prefix('{').and_then(|r| r.split_once('}')) {
        Some((g, rest)) => format!("{}{}", known_folder(&format!("{{{g}}}"))?, rest),
        None => name.to_string(),
    };
    let lower = path.to_lowercase();
    if !lower.ends_with(".exe") {
        return None;
    }
    let runs = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let ft = u64::from_le_bytes(data[60..68].try_into().ok()?);
    Some(Launch {
        path,
        runs,
        last: filetime_to_unix(ft),
    })
}

const USERASSIST: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist";

/// Programs started from Explorer, the Start menu and the taskbar.
pub fn userassist(reg: &dyn Registry) -> Vec<Launch> {
    let mut out = Vec::new();
    for g in reg.subkeys(USERASSIST) {
        let Some(k) = reg.read_key(&format!("{USERASSIST}\\{g}\\Count")) else {
            continue;
        };
        for v in k.values.values() {
            if let Some(l) = decode_userassist(&rot13(&v.name), &v.data) {
                out.push(l);
            }
        }
    }
    out
}

/// Last run time by program file name, from Windows' Prefetch folder
/// (readable with administrator rights).
pub fn prefetch() -> HashMap<String, i64> {
    let mut out: HashMap<String, i64> = HashMap::new();
    let dir = PathBuf::from(std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("Prefetch");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let Some(stem) = name.strip_suffix(".pf") else {
            continue;
        };
        let Some((exe, _hash)) = stem.rsplit_once('-') else {
            continue;
        };
        let Some(t) = e.metadata().ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        let secs = t
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let slot = out.entry(exe.to_string()).or_insert(0);
        *slot = (*slot).max(secs);
    }
    out
}

// ---- Memory and CPU over time ------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExeStats {
    pub samples: u32,
    /// Sum of working sets (bytes) over samples, for the average.
    pub ws_sum: u64,
    pub ws_peak: u64,
    /// CPU time used while sampled, and the wall time covered.
    pub cpu_ms: u64,
    pub wall_ms: u64,
    pub last_cpu_ms: u64,
    pub last_seen: i64,
}
crate::json_struct!(ExeStats, { samples, ws_sum, ws_peak, cpu_ms, wall_ms, last_cpu_ms [default], last_seen });

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageDb {
    /// By lower-case program file path.
    pub exes: HashMap<String, ExeStats>,
    pub last_sample: i64,
}
crate::json_struct!(UsageDb, { exes, last_sample });

/// One running process.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcSample {
    pub path: String,
    pub working_set: u64,
    pub cpu_ms: u64,
}

impl UsageDb {
    pub fn path(root: &Path) -> PathBuf {
        root.join("usage.json")
    }

    pub fn load(root: &Path) -> Self {
        std::fs::read(Self::path(root))
            .ok()
            .and_then(|b| crate::json::compat::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) {
        if let Ok(b) = crate::json::compat::to_vec(self) {
            let tmp = root.join("usage.json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path(root));
            }
        }
    }

    /// Add a round of samples taken at `now` (Unix seconds).
    pub fn record(&mut self, samples: &[ProcSample], now: i64) {
        let gap_ms = if self.last_sample > 0 {
            ((now - self.last_sample).clamp(0, 3600) * 1000) as u64
        } else {
            0
        };
        // Several processes of one program (browsers) add up.
        let mut by_exe: HashMap<String, (u64, u64)> = HashMap::new();
        for s in samples {
            let e = by_exe.entry(s.path.to_lowercase()).or_default();
            e.0 += s.working_set;
            e.1 += s.cpu_ms;
        }
        for (exe, (ws, cpu)) in by_exe {
            let st = self.exes.entry(exe).or_default();
            st.samples += 1;
            st.ws_sum += ws;
            st.ws_peak = st.ws_peak.max(ws);
            if st.last_seen > 0 && cpu >= st.last_cpu_ms && gap_ms > 0 {
                st.cpu_ms += cpu - st.last_cpu_ms;
                st.wall_ms += gap_ms;
            }
            st.last_cpu_ms = cpu;
            st.last_seen = now;
        }
        self.last_sample = now;
    }
}

/// Take a round of samples (Windows); nothing elsewhere.
pub fn sample_now() -> Vec<ProcSample> {
    #[cfg(windows)]
    {
        crate::platform::windows::process_samples()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Sample and save, if the last round was at least `min_gap` seconds ago.
pub fn sample_and_save(root: &Path, min_gap: i64) {
    let now = crate::time::unix_now();
    let mut db = UsageDb::load(root);
    if now - db.last_sample < min_gap {
        return;
    }
    let s = sample_now();
    if s.is_empty() {
        return;
    }
    db.record(&s, now);
    db.save(root);
}

// ---- Per program -------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProgramUsage {
    /// When it was last opened (Unix seconds), if Windows recorded it.
    pub last_opened: Option<i64>,
    pub runs: Option<u32>,
    pub avg_ram: Option<u64>,
    pub peak_ram: Option<u64>,
    /// Average CPU while running, in percent of one core.
    pub cpu_percent: Option<f32>,
    /// How much it slows startup (ms), from Windows' boot diagnostics.
    pub startup_ms: Option<u64>,
}

/// Everything needed to work out usage for many programs at once.
#[derive(Clone, Debug, Default)]
pub struct UsageData {
    pub launches: Vec<Launch>,
    pub prefetch: HashMap<String, i64>,
    pub db: UsageDb,
    /// Startup delay by lower-case program file name.
    pub startup: HashMap<String, u64>,
}

impl UsageData {
    pub fn gather(reg: &dyn Registry, root: &Path) -> Self {
        let startup = crate::boot::read()
            .map(|b| b.apps.iter().map(|(k, v)| (k.clone(), v.avg_ms)).collect())
            .unwrap_or_default();
        UsageData {
            launches: userassist(reg),
            prefetch: prefetch(),
            db: UsageDb::load(root),
            startup,
        }
    }

    /// Usage of one program: its program files are the ones in its folder.
    pub fn for_program(&self, p: &InstalledProgram) -> ProgramUsage {
        let mut u = ProgramUsage::default();
        let loc = p
            .install_location
            .trim()
            .trim_end_matches('\\')
            .to_lowercase();
        if loc.len() < 4 {
            return u;
        }
        let inside = |path: &str| {
            let l = path.to_lowercase();
            l.len() > loc.len() && l.starts_with(&loc) && l.as_bytes()[loc.len()] == b'\\'
        };
        for l in self.launches.iter().filter(|l| inside(&l.path)) {
            u.runs = Some(u.runs.unwrap_or(0) + l.runs);
            if let Some(t) = l.last {
                u.last_opened = Some(u.last_opened.map_or(t, |x| x.max(t)));
            }
        }
        let (mut ws_sum, mut samples, mut peak, mut cpu, mut wall) = (0u64, 0u32, 0u64, 0u64, 0u64);
        let mut exe_names: Vec<String> = Vec::new();
        for (exe, st) in self.db.exes.iter().filter(|(e, _)| inside(e)) {
            ws_sum += st.ws_sum;
            samples += st.samples;
            peak = peak.max(st.ws_peak);
            cpu += st.cpu_ms;
            wall += st.wall_ms;
            if st.last_seen > 0 {
                u.last_opened = Some(u.last_opened.map_or(st.last_seen, |x| x.max(st.last_seen)));
            }
            exe_names.push(exe.rsplit('\\').next().unwrap_or(exe).to_string());
        }
        if samples > 0 {
            u.avg_ram = Some(ws_sum / samples as u64);
            u.peak_ram = Some(peak);
        }
        if wall > 0 {
            u.cpu_percent = Some(cpu as f32 * 100.0 / wall as f32);
        }
        // Prefetch knows program file names only: use the ones seen in the
        // program's folder, or the one its icon points to.
        let icon = p
            .display_icon
            .split(',')
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .to_lowercase();
        if inside(&icon) {
            if let Some(n) = icon.rsplit('\\').next() {
                exe_names.push(n.to_string());
            }
        }
        for n in &exe_names {
            if let Some(&t) = self.prefetch.get(n) {
                u.last_opened = Some(u.last_opened.map_or(t, |x| x.max(t)));
            }
            if let Some(&ms) = self.startup.get(n) {
                u.startup_ms = Some(u.startup_ms.unwrap_or(0) + ms);
            }
        }
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot13_round_trip() {
        assert_eq!(
            rot13("P:\\Cebtenz Svyrf\\Nccf\\nccf.rkr"),
            "C:\\Program Files\\Apps\\apps.exe"
        );
        assert_eq!(rot13(&rot13("Hello")), "Hello");
    }

    #[test]
    fn decodes_userassist_entries() {
        let mut data = vec![0u8; 72];
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        // 2026-01-01 00:00:00 UTC as FILETIME.
        let ft: u64 = (1_767_225_600 + 11_644_473_600) * 10_000_000;
        data[60..68].copy_from_slice(&ft.to_le_bytes());
        let l = decode_userassist("C:\\Tools\\App\\app.exe", &data).unwrap();
        assert_eq!((l.runs, l.last), (7, Some(1_767_225_600)));
        assert!(decode_userassist("C:\\Tools\\App\\readme.txt", &data).is_none());
        assert!(decode_userassist("C:\\x.exe", &data[..10]).is_none());
    }

    #[test]
    fn records_memory_and_cpu() {
        let mut db = UsageDb::default();
        let s = |cpu| {
            vec![
                ProcSample {
                    path: "C:\\App\\app.exe".into(),
                    working_set: 100,
                    cpu_ms: cpu,
                },
                ProcSample {
                    path: "c:\\app\\APP.exe".into(),
                    working_set: 50,
                    cpu_ms: 0,
                },
            ]
        };
        db.record(&s(1000), 1_000);
        db.record(&s(4000), 1_060);
        let st = &db.exes["c:\\app\\app.exe"];
        assert_eq!((st.samples, st.ws_peak, st.ws_sum), (2, 150, 300));
        assert_eq!((st.cpu_ms, st.wall_ms), (3000, 60_000));
        let data = UsageData {
            db,
            ..Default::default()
        };
        let p = InstalledProgram {
            install_location: "C:\\App".into(),
            ..Default::default()
        };
        let u = data.for_program(&p);
        assert_eq!(u.avg_ram, Some(150));
        assert_eq!(u.cpu_percent, Some(5.0));
        assert_eq!(u.last_opened, Some(1_060));
    }
}
