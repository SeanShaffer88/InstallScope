//! Bounded, offline pattern triage. Log text is untrusted data, never instructions.
use crate::{
    devcheck::{probe, Probe},
    snapshot::Progress,
};
use anyhow::{bail, ensure, Result};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LINES: usize = 20_000;
pub const MAX_LINE_BYTES: usize = 8192;
pub struct Rule {
    pub id: &'static str,
    pub title: &'static str,
    pub hint: &'static str,
    pub url: &'static str,
    needles: &'static [&'static str],
}
pub const RULES: &[Rule] = &[
    Rule { id:"rust-target", title:"Rust target/core library unavailable", needles:&["e0463"], hint:"Check the selected toolchain, target and installed standard libraries. A missing crate can have other causes; inspect the original diagnostic.", url:"https://doc.rust-lang.org/error_codes/E0463.html" },
    Rule { id:"rust-import", title:"Rust unresolved import", needles:&["e0432"], hint:"Check module paths, dependency declarations, features and edition for the selected target.", url:"https://doc.rust-lang.org/error_codes/E0432.html" },
    Rule { id:"rust-linker", title:"Rust linker could not be located", needles:&["linker `link.exe` not found", "linker `cc` not found"], hint:"Check the intended native toolchain and inherited developer environment. Do not assume a PATH candidate is usable.", url:"https://rust-lang.github.io/rustup/installation/windows.html" },
    Rule { id:"lnk1104", title:"MSVC linker cannot open a file", needles:&["lnk1104"], hint:"Review the named input/output in the original log, library search paths, permissions and possible file holders. Do not automatically delete or unlock it.", url:"https://learn.microsoft.com/en-us/cpp/error-messages/tool-errors/linker-tools-error-lnk1104" },
    Rule { id:"lnk2019", title:"MSVC unresolved external symbol", needles:&["lnk2019", "lnk2001"], hint:"Compare declarations, linked libraries, target architecture and build configuration. An unresolved symbol does not identify a single cause.", url:"https://learn.microsoft.com/en-us/cpp/error-messages/tool-errors/linker-tools-error-lnk2019" },
    Rule { id:"msvc-header", title:"MSVC cannot open an include/source file", needles:&["c1083"], hint:"Review include paths, generated files, SDK selection and the original filename; avoid copying unknown DLLs or headers into system folders.", url:"https://learn.microsoft.com/en-us/cpp/error-messages/compiler-errors-1/fatal-error-c1083" },
    Rule { id:"msbuild-toolset", title:"MSBuild platform toolset unavailable", needles:&["msb8020"], hint:"Compare the project's requested toolset with installed Visual Studio components and the selected developer environment.", url:"https://learn.microsoft.com/en-us/visualstudio/msbuild/errors/msb8020" },
    Rule { id:"dotnet-sdk", title:"Selected .NET SDK does not support the target", needles:&["netsdk1045"], hint:"Compare TargetFramework, global.json and available SDK architectures/versions. Do not silently retarget the project.", url:"https://learn.microsoft.com/en-us/dotnet/core/tools/sdk-errors/netsdk1045" },
    Rule { id:"nuget-restore", title:"NuGet service index unavailable", needles:&["nu1301"], hint:"Review the original feed error, network/proxy setup and authentication locally. Never paste credentials into reports or disable TLS checks.", url:"https://learn.microsoft.com/en-us/nuget/reference/errors-and-warnings/nu1301" },
    Rule { id:"typescript-module", title:"TypeScript module or declaration unavailable", needles:&["ts2307"], hint:"Review the import, installed dependencies, types and module-resolution configuration for this workspace.", url:"https://www.typescriptlang.org/docs/handbook/modules/reference.html" },
    Rule { id:"npm-resolution", title:"npm dependency resolution conflict", needles:&["eresolve"], hint:"Compare peer dependency requirements and lockfile provenance. Do not automatically force installation or bypass dependency checks.", url:"https://docs.npmjs.com/cli/using-npm/config" },
    Rule { id:"python-module", title:"Python import dependency unavailable", needles:&["modulenotfounderror:"], hint:"Check the intended interpreter/virtual environment and project dependency declarations. A package name is not necessarily the import name.", url:"https://docs.python.org/3/library/exceptions.html#ModuleNotFoundError" },
    Rule { id:"cmake-compiler", title:"CMake compiler discovery failed", needles:&["no cmake_c_compiler could be found", "no cmake_cxx_compiler could be found"], hint:"Compare generator, compiler/toolchain selection, target architecture and the developer shell. Existing cache settings may matter.", url:"https://cmake.org/cmake/help/latest/manual/cmake-toolchains.7.html" },
    Rule { id:"disk-full", title:"Possible storage exhaustion", needles:&["no space left on device", "enospc", "there is not enough space on the disk"], hint:"Inspect the build, temporary and output volumes. Review storage attribution; this tool does not remove caches or files.", url:"https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-" },
    Rule { id:"access-denied", title:"Possible access/permission failure", needles:&["permission denied", "access is denied", "eacces"], hint:"Review the original failing resource and its ownership/holders. Do not automatically elevate, alter permissions or disable security controls.", url:"https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-" },
];
#[derive(Debug)]
pub struct Finding {
    pub rule: usize,
    pub occurrences: usize,
    pub lines: Vec<usize>,
}
#[derive(Debug)]
pub struct Report {
    pub path: PathBuf,
    pub observed: String,
    pub bytes: usize,
    pub lines: usize,
    pub skipped_long_lines: usize,
    pub partial: bool,
    pub cancelled: bool,
    pub findings: Vec<Finding>,
}
impl Report {
    pub fn text(&self) -> String {
        let mut out = format!("Build-log triage (heuristic, read-only)\nSource: {}\nObserved: {}\nBytes: {} · Lines inspected: {} · Long lines skipped: {}\nPartial: {} · Cancelled: {}\nNo build executed; no raw log lines copied. Matches may be incidental, quoted or downstream symptoms. No matches does not prove success.\n", self.path.display(), self.observed, self.bytes, self.lines, self.skipped_long_lines, self.partial, self.cancelled);
        for finding in &self.findings {
            let rule = &RULES[finding.rule];
            out.push_str(&format!(
                "\n{} — {} occurrence(s); first source lines {:?}\n{}\n{}\n",
                rule.title, finding.occurrences, finding.lines, rule.hint, rule.url
            ));
        }
        if self.findings.is_empty() {
            out.push_str("\nNo supported patterns recognized in the inspected portion. Review the original log.\n");
        }
        out
    }
}
fn decode(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        ensure!(
            bytes.len().is_multiple_of(2),
            "Malformed UTF-16: incomplete code unit."
        );
        let le = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if le {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        Ok(String::from_utf16(&units)?)
    } else {
        Ok(
            std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?
                .to_owned(),
        )
    }
}
fn normalize(line: &str) -> String {
    // Strip bounded ANSI CSI/OSC sequences without interpreting links/commands.
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() || c == '\t' {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}
fn token_match(line: &str, needle: &str) -> bool {
    line.match_indices(needle).any(|(i, _)| {
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        (i == 0 || !word(line.as_bytes()[i - 1]))
            && (i + needle.len() == line.len() || !word(line.as_bytes()[i + needle.len()]))
    })
}
fn analyze(path: &Path, bytes: &[u8], progress: &Progress) -> Result<Report> {
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Log exceeds 2 MiB. Select a smaller excerpt; the original file is unchanged."
    );
    let text = decode(bytes)?;
    ensure!(
        !text.contains('\0'),
        "Binary/NUL-containing input is unsupported. Select UTF-8 or BOM-marked UTF-16 text."
    );
    let mut report = Report {
        path: path.into(),
        observed: crate::util::now_string(),
        bytes: bytes.len(),
        lines: 0,
        skipped_long_lines: 0,
        partial: false,
        cancelled: false,
        findings: Vec::new(),
    };
    for (index, line) in text.lines().enumerate() {
        if index >= MAX_LINES || progress.cancelled() {
            report.partial = true;
            break;
        }
        progress.tick();
        report.lines += 1;
        if line.len() > MAX_LINE_BYTES {
            report.skipped_long_lines += 1;
            report.partial = true;
            continue;
        }
        let line = normalize(line);
        for (rule, definition) in RULES.iter().enumerate() {
            if definition
                .needles
                .iter()
                .any(|needle| token_match(&line, needle))
            {
                if let Some(f) = report.findings.iter_mut().find(|f| f.rule == rule) {
                    f.occurrences += 1;
                    if f.lines.len() < 5 {
                        f.lines.push(index + 1);
                    }
                } else {
                    report.findings.push(Finding {
                        rule,
                        occurrences: 1,
                        lines: vec![index + 1],
                    });
                }
            }
        }
    }
    report.cancelled = progress.cancelled();
    report.partial |= report.cancelled;
    Ok(report)
}
pub fn inspect(path: &Path, progress: &Progress) -> Result<Report> {
    progress.set_phase("Reading bounded build log");
    match probe(path) {
        Probe::File => {}
        other => bail!("Choose a regular local non-reparse text file: {other:?}"),
    }
    let file = File::open(path)?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= MAX_BYTES as u64,
        "Log exceeds 2 MiB or is not regular. Select a smaller text excerpt."
    );
    let mut bytes = Vec::new();
    (&file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "File changed during reading. Select a stable saved log and retry."
    );
    analyze(path, &bytes, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(bytes: &[u8]) -> Report {
        analyze(Path::new("fixture.log"), bytes, &Progress::default()).unwrap()
    }
    #[test]
    fn colored_codes_grouped_with_source_lines_and_no_raw_secrets() {
        let r = run(b"SECRET_TOKEN=private\r\n\x1b[31merror LNK1104\x1b[0m\nLINK : LNK1104\nerror E0432\nLNK11040 should not match\n");
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.findings[0].occurrences, 2);
        assert_eq!(r.findings[0].lines, vec![2, 3]);
        assert!(!r.text().contains("SECRET_TOKEN"));
        assert!(!r.text().contains("private"));
    }
    #[test]
    fn unicode_encodings_and_binary_rejection() {
        for le in [true, false] {
            let mut bytes = if le {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for c in "令牌 error NETSDK1045".encode_utf16() {
                bytes.extend(if le { c.to_le_bytes() } else { c.to_be_bytes() });
            }
            assert_eq!(run(&bytes).findings.len(), 1);
        }
        assert_eq!(run(b"\xef\xbb\xbfTS2307").findings.len(), 1);
        for input in [
            &b"\xff"[..],
            &b"binary\0input"[..],
            &b"\xff\xfe\0"[..],
            &b"\xff\xfe\0\xd8"[..],
        ] {
            assert!(analyze(Path::new("x"), input, &Progress::default()).is_err());
        }
    }
    #[test]
    fn bounds_and_cancel_are_explicit() {
        assert!(analyze(
            Path::new("x"),
            &vec![b'x'; MAX_BYTES + 1],
            &Progress::default()
        )
        .is_err());
        let r = run(format!("{}\nLNK2019", "x".repeat(MAX_LINE_BYTES + 1)).as_bytes());
        assert!(r.partial);
        assert_eq!(r.skipped_long_lines, 1);
        assert_eq!(r.findings[0].lines, vec![2]);
        let r = run("LNK2019\n".repeat(MAX_LINES + 1).as_bytes());
        assert!(r.partial);
        assert_eq!(r.lines, MAX_LINES);
        assert_eq!(r.findings[0].lines.len(), 5);
        let p = Progress::default();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let r = analyze(Path::new("x"), b"LNK1104", &p).unwrap();
        assert!(r.cancelled);
        assert!(r.findings.is_empty());
    }
    #[test]
    fn patterns_are_hints_and_terminal_controls_are_not_content() {
        assert!(run(b"all tests passed").findings.is_empty());
        assert!(run(b"\x1b]8;;https://secret/ERESOLVE\x07\x1b]8;;\x1b\\")
            .findings
            .is_empty());
        for rule in RULES {
            for needle in rule.needles {
                assert!(token_match(&normalize(needle), needle), "{}", rule.id);
            }
        }
    }
}
