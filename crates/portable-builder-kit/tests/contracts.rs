use portable_builder_kit::*;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "pafkit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, bytes).unwrap();
        p
    }
    fn watch(&self) -> ArtifactWatch {
        ArtifactWatch::capture(std::slice::from_ref(&self.0), "DemoPortable", &fingerprint).unwrap()
    }
    fn package(&self) {
        self.write("App/AppInfo/appinfo.ini", b"[Format]\r\nType=PortableApps.comFormat\r\nVersion=3.9\r\n[Details]\r\nAppID=DemoPortable\r\n[Control]\r\nStart=DemoPortable.exe\r\n");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Test-only fingerprint. Consumers must supply a real complete-file SHA-256.
fn fingerprint(p: &Path) -> io::Result<Sha256> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fs::read(p)?.hash(&mut h);
    let mut out = [0; 32];
    out[..8].copy_from_slice(&h.finish().to_le_bytes());
    Ok(out)
}
fn pe(marker: u8) -> Vec<u8> {
    let mut v = vec![0; 128];
    v[..2].copy_from_slice(b"MZ");
    v[60..64].copy_from_slice(&64u32.to_le_bytes());
    v[64..68].copy_from_slice(b"PE\0\0");
    v[127] = marker;
    v
}

#[test]
fn rejects_windows_aliases_and_traversal_on_every_platform() {
    for p in [
        "",
        ".",
        "..",
        "NUL",
        "nul.txt",
        "COM1.exe",
        "LPT³.txt",
        "CONIN$",
        "CON .txt",
        "hello.",
        "hello ",
        "a:b",
        "a\nb",
        "C:\\escape",
        "\\\\server\\share",
        "a/../b",
        "/root",
        "a//b",
    ] {
        assert!(relative_path(p).is_err(), "accepted {p:?}");
    }
    assert_eq!(
        relative_path("工具\\app.exe").unwrap(),
        Path::new("工具").join("app.exe")
    );
}
#[test]
fn validates_ids() {
    for id in ["", "..", "a/b", "a\\b", "NUL", "_bad", "éPortable"] {
        assert!(validate_app_id(id).is_err());
    }
    assert!(validate_app_id("Demo++Portable-example.org").is_ok());
}
#[test]
fn validates_pe_signatures_and_offsets() {
    let f = Fixture::new();
    let p = f.write("valid.exe", &pe(1));
    validate_executable(&p).unwrap();
    for bytes in [
        b"MZ".to_vec(),
        vec![0; 128],
        {
            let mut v = pe(1);
            v[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
            v
        },
        {
            let mut v = pe(1);
            v[64] = 0;
            v
        },
    ] {
        fs::write(&p, bytes).unwrap();
        assert!(validate_executable(&p).is_err());
    }
}
#[test]
fn unchanged_recent_artifact_is_not_success() {
    let f = Fixture::new();
    f.write("DemoPortable_1.paf.exe", &pe(1));
    assert!(f.watch().finish(0, &fingerprint).is_err());
}
#[test]
fn changed_content_is_detected_at_same_path() {
    let f = Fixture::new();
    let p = f.write("DemoPortable_1.paf.exe", &pe(1));
    let w = f.watch();
    fs::write(&p, pe(2)).unwrap();
    let a = w.finish(0, &fingerprint).unwrap();
    assert_eq!(a.path, p.canonicalize().unwrap());
    assert_eq!(a.sha256, fingerprint(&p).unwrap());
}
#[test]
fn unrelated_artifacts_are_ignored() {
    let f = Fixture::new();
    let w = f.watch();
    f.write("OtherPortable_1.paf.exe", &pe(1));
    assert!(w.finish(0, &fingerprint).is_err());
}
#[test]
fn nonzero_exit_cannot_be_masked_by_output() {
    let f = Fixture::new();
    let w = f.watch();
    f.write("DemoPortable_1.paf.exe", &pe(1));
    assert!(w
        .finish(7, &fingerprint)
        .unwrap_err()
        .to_string()
        .contains("exit code 7"));
}
#[test]
fn multiple_changed_artifacts_are_ambiguous() {
    let f = Fixture::new();
    let w = f.watch();
    f.write("DemoPortable_1.paf.exe", &pe(1));
    f.write("DemoPortable_2.paf.exe", &pe(2));
    assert!(w.finish(0, &fingerprint).is_err());
}
#[test]
fn missing_directory_is_not_silently_ignored() {
    let f = Fixture::new();
    assert!(ArtifactWatch::capture(&[f.0.join("missing")], "DemoPortable", &fingerprint).is_err());
}
#[test]
fn duplicate_directory_does_not_duplicate_artifact() {
    let f = Fixture::new();
    let w =
        ArtifactWatch::capture(&[f.0.clone(), f.0.clone()], "DemoPortable", &fingerprint).unwrap();
    f.write("DemoPortable_1.paf.exe", &pe(1));
    w.finish(0, &fingerprint).unwrap();
}
#[test]
fn hash_failure_is_fatal() {
    let f = Fixture::new();
    f.write("DemoPortable_1.paf.exe", &pe(1));
    let hash = |_: &Path| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unreadable",
        ))
    };
    assert!(ArtifactWatch::capture(std::slice::from_ref(&f.0), "DemoPortable", &hash).is_err());
}
#[test]
fn corrupt_artifact_is_not_success() {
    let f = Fixture::new();
    let w = f.watch();
    f.write("DemoPortable_1.paf.exe", b"compiler log, not executable");
    assert!(w.finish(0, &fingerprint).is_err());
}
#[test]
fn reads_bom_utf8_and_utf16() {
    let f = Fixture::new();
    f.package();
    let p = f.0.join("App/AppInfo/appinfo.ini");
    let text = fs::read_to_string(&p).unwrap();
    let mut bytes = vec![239, 187, 191];
    bytes.extend_from_slice(text.as_bytes());
    fs::write(&p, bytes).unwrap();
    assert_eq!(package_app_id(&f.0).unwrap(), "DemoPortable");
    let mut bytes = vec![255, 254];
    for u in text.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    fs::write(&p, bytes).unwrap();
    assert_eq!(package_app_id(&f.0).unwrap(), "DemoPortable");
}
#[test]
fn rejects_ambiguous_and_malformed_metadata() {
    let f = Fixture::new();
    f.package();
    let p = f.0.join("App/AppInfo/appinfo.ini");
    let good = fs::read_to_string(&p).unwrap();
    for bad in [
        good.replace("AppID=DemoPortable", "AppID=../escape"),
        good.replace("Start=DemoPortable.exe", "Start=Other.exe"),
        good.clone() + "Start=Other.exe\n",
        good.clone() + "[details]\nAppID=Other\n",
        good.clone() + "bad line\n",
        good.clone() + "\0",
        "x".repeat(65537),
    ] {
        fs::write(&p, bad).unwrap();
        assert!(package_app_id(&f.0).is_err());
    }
}
#[test]
fn preflight_reports_all_missing_assets() {
    let f = Fixture::new();
    f.package();
    assert_eq!(preflight(&f.0, false).len(), 6);
    for p in [
        "help.html",
        "App/AppInfo/appicon.ico",
        "App/AppInfo/appicon_16.png",
        "App/AppInfo/appicon_32.png",
        "App/AppInfo/appicon_128.png",
        "App/AppInfo/Launcher/DemoPortable.ini",
    ] {
        f.write(p, b"fixture");
    }
    assert!(preflight(&f.0, false).is_empty());
    assert_eq!(preflight(&f.0, true).len(), 1);
    f.write("DemoPortable.exe", &pe(1));
    assert!(preflight(&f.0, true).is_empty());
}
#[cfg(unix)]
#[test]
fn linked_artifact_is_rejected() {
    let f = Fixture::new();
    let p = f.write("target", &pe(1));
    std::os::unix::fs::symlink(p, f.0.join("DemoPortable_1.paf.exe")).unwrap();
    assert!(
        ArtifactWatch::capture(std::slice::from_ref(&f.0), "DemoPortable", &fingerprint).is_err()
    );
}

#[test]
fn identical_rebuild_with_updated_modification_time_is_accepted() {
    use std::time::{Duration, SystemTime};
    let f = Fixture::new();
    let p = f.write("DemoPortable_1.paf.exe", &pe(1));
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1000))
        .unwrap();
    let watch = f.watch();
    fs::write(&p, pe(1)).unwrap();
    watch.finish(0, &fingerprint).unwrap();
}
