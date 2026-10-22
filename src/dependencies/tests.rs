use super::*;
use std::cell::Cell;
fn fixture(root: &Path) -> (Package, InstallReview) {
    fs::create_dir_all(root).unwrap();
    let path = root.join("fixture.exe");
    fs::write(&path, b"abc").unwrap();
    let p = Package {
        id: "fixture".into(),
        name: "Fixture".into(),
        version: "1".into(),
        arch: "x64".into(),
        filename: "fixture.exe".into(),
        url: String::new(),
        bytes: 3,
        algorithm: "SHA256".into(),
        digest: util::sha256(b"abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        publisher: "Fixture Publisher".into(),
        kind: "exe".into(),
        source: String::new(),
        license_url: String::new(),
        notes: String::new(),
    };
    let review = InstallReview {
        id: p.id.clone(),
        path: path.clone(),
        elevated: false,
        text: "exact fixture review".into(),
        _held: lock(&path).unwrap(),
        command: None,
    };
    (p, review)
}
struct FakeProcess(u32);
impl Running for FakeProcess {
    fn pid(&self) -> u32 {
        123
    }
    fn wait(&self) -> Result<u32> {
        Ok(self.0)
    }
}
fn trusted(_: &Package, _: &Path) -> Result<String> {
    Ok("Fixture Publisher".into())
}

#[test]
fn trusted_catalog_is_bounded_unique_and_offline() {
    let c = catalog().unwrap();
    assert_eq!(c.packages.len(), 6);
    for p in &c.packages {
        assert!(safe_name(&p.filename));
        assert!(p.notes.contains("installer") || p.notes.contains("compiler"));
    }
    assert!(selection(&[]).is_err());
    assert!(selection(&["../../evil".into()]).is_err());
    let id = &c.packages[0].id;
    assert!(selection(&[id.clone(), id.clone()]).is_err());
}
#[test]
fn https_and_redirect_hosts_are_constrained() {
    for url in [
        "http://github.com/Kitware/CMake/releases/download/v/a",
        "https://github.com.evil/Kitware/CMake/releases/download/a",
        "https://user@github.com/Kitware/CMake/releases/download/a",
        "https://github.com:443/Kitware/CMake/releases/download/a",
        "https://github.com/other/repo",
        "https://builds.dotnet.microsoft.com/other/a",
        "https://github.com/Kitware/CMake/releases/download/a#secret",
        "https://release-assets.githubusercontent.com/\r\na",
    ] {
        assert!(allowed_url(url).is_err(), "{url:?}");
    }
    assert!(allowed_url("https://release-assets.githubusercontent.com/a?token=opaque").is_ok());
}
#[test]
fn paths_reject_relative_traversal_and_devices() {
    assert!(local_path(Path::new("relative.exe")).is_err());
    let t = util::tempdir().unwrap();
    assert!(local_path(&t.path().join("..\\escape")).is_err() || !cfg!(windows));
    #[cfg(windows)]
    for p in [
        r"\\server\share\a",
        r"\\?\C:\a",
        r"C:\a:stream",
        r"C:\a\..\b",
    ] {
        assert!(local_path(Path::new(p)).is_err());
    }
}
#[test]
fn hashing_rejects_truncated_and_changed_content() {
    let t = util::tempdir().unwrap();
    let (p, mut review) = fixture(t.path());
    let progress = Progress::default();
    check_bytes(&p, &mut review._held.file, &progress).unwrap();
    let mut wrong = p.clone();
    wrong.bytes = 4;
    assert!(check_bytes(&wrong, &mut review._held.file, &progress).is_err());
    wrong = p;
    wrong.digest = "0".repeat(64);
    assert!(check_bytes(&wrong, &mut review._held.file, &progress).is_err());
}
#[test]
fn streaming_hash_vectors_and_cancellation() {
    let progress = Progress::default();
    assert_eq!(
        digest(&mut &b"abc"[..], "SHA256", &progress).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    #[cfg(windows)]
    assert_eq!(digest(&mut &b"abc"[..],"SHA512",&progress).unwrap(),"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
    let b = vec![7u8; 131073];
    let expected: String = util::sha256(&b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest(&mut &b[..], "SHA256", &progress).unwrap(), expected);
    progress
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(digest(&mut &b[..], "SHA256", &progress).is_err());
}
#[test]
fn foreign_manifest_fields_and_unknown_packages_are_rejected() {
    let t = util::tempdir().unwrap();
    let c = catalog().unwrap();
    let id = &c.packages[0].id;
    let valid = Manifest {
        schema: 1,
        catalog: c.verified.clone(),
        packages: vec![id.clone()],
    };
    let path = t.path().join("bundle.json");
    fs::write(&path, json::to_json_string_pretty(&valid)).unwrap();
    assert!(read_manifest(t.path()).is_ok());
    for bad in [
        format!(
            "{{\"schema\":1,\"catalog\":\"{}\",\"packages\":[\"{}\"],\"url\":\"https://evil\"}}",
            c.verified, id
        ),
        format!(
            "{{\"schema\":1,\"catalog\":\"{}\",\"packages\":[\"../evil\"]}}",
            c.verified
        ),
        "{\"schema\":1,\"schema\":1,\"catalog\":\"x\",\"packages\":[]}".into(),
        "{".repeat(400),
        " ".repeat(32769),
    ] {
        fs::write(&path, bad).unwrap();
        assert!(read_manifest(t.path()).is_err());
    }
}
#[test]
fn incomplete_bundle_never_creates_cache_and_export_requires_license_review() {
    let t = util::tempdir().unwrap();
    let source = t.path().join("incomplete");
    fs::create_dir(&source).unwrap();
    let root = t.path().join("library");
    assert!(import_bundle(&root, &source, &Progress::default()).is_err());
    assert!(!root.exists());
    assert!(export_bundle(
        &root,
        &[catalog().unwrap().packages[0].id.clone()],
        &t.path().join("new"),
        false,
        &Progress::default()
    )
    .is_err());
    assert!(!t.path().join("new").exists());
}
#[test]
fn no_review_no_signature_no_receipt_or_cancel_means_no_launch() {
    for failure in ["review", "signature", "receipt", "cancel", "hash"] {
        let t = util::tempdir().unwrap();
        let root = t.path().join("root");
        let (mut p, review) = fixture(&t.path().join("installer"));
        let progress = Progress::default();
        let launches = Cell::new(0);
        if failure == "receipt" {
            fs::create_dir_all(root.join("dependencies")).unwrap();
            fs::write(root.join("dependencies/receipts"), b"blocked").unwrap();
        }
        if failure == "cancel" {
            progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if failure == "hash" {
            p.digest = "0".repeat(64);
        }
        let result = install_with(
            &root,
            review,
            failure != "review",
            &progress,
            p,
            |p, path| {
                if failure == "signature" {
                    bail!("untrusted");
                }
                trusted(p, path)
            },
            |_, _, _| {
                launches.set(launches.get() + 1);
                Ok(FakeProcess(0))
            },
        );
        assert!(result.is_err(), "{failure}");
        assert_eq!(launches.get(), 0, "{failure}");
    }
}
#[test]
fn reviewed_mock_install_records_reboot_and_vendor_cancel_without_claiming_readiness() {
    for code in [0, 3010, 1602, 77] {
        let t = util::tempdir().unwrap();
        let root = t.path().join("library");
        let (p, review) = fixture(&t.path().join("installer"));
        let calls = Cell::new(0);
        let result = install_with(
            &root,
            review,
            true,
            &Progress::default(),
            p,
            trusted,
            |_, _, elevated| {
                assert!(!elevated);
                assert_eq!(
                    recent_receipts(&root).unwrap()[0].state,
                    "prepared; launch not yet confirmed"
                );
                calls.set(calls.get() + 1);
                Ok(FakeProcess(code))
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        let receipts = recent_receipts(&root).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].exit_code, Some(code));
        assert_eq!(receipts[0].pid, 123);
        assert!(result.contains("Reinspect"));
        if code == 3010 {
            assert!(receipts[0].state.contains("reboot"));
        }
    }
}
#[test]
fn declined_launch_keeps_an_honest_failure_receipt() {
    let t = util::tempdir().unwrap();
    let root = t.path().join("library");
    let (p, review) = fixture(&t.path().join("installer"));
    let result = install_with::<FakeProcess>(
        &root,
        review,
        true,
        &Progress::default(),
        p,
        trusted,
        |_, _, _| bail!("user declined"),
    );
    assert!(result.is_err());
    let r = recent_receipts(&root).unwrap();
    assert_eq!(r[0].pid, 0);
    assert_eq!(r[0].exit_code, None);
    assert!(r[0].state.contains("declined"));
}
#[cfg(windows)]
#[test]
fn held_review_blocks_writes_and_file_replacement() {
    let t = util::tempdir().unwrap();
    let (_, review) = fixture(t.path());
    assert!(OpenOptions::new().write(true).open(&review.path).is_err());
    assert!(fs::remove_file(&review.path).is_err());
    let target = t.path().with_extension("moved");
    assert!(fs::rename(t.path(), &target).is_err());
    drop(review);
    assert!(OpenOptions::new()
        .write(true)
        .open(t.path().join("fixture.exe"))
        .is_ok());
}
