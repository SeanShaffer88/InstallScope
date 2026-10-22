//! Dedicated development helper: redirects only this process's HKCU to a new
//! fixture key before invoking the native environment mutation adapter.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use setupscope_core::{
        envprofile::{self, Environment, Profile, UserEnvironment, Variable},
        model::{reg_type, RegValue},
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::System::Registry::*;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    struct Mapping(bool);
    impl Mapping {
        fn restore(&mut self) -> anyhow::Result<()> {
            if self.0 {
                let rc = unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, std::ptr::null_mut()) };
                ensure!(rc == 0, "Cannot restore helper HKCU mapping: {rc}");
                self.0 = false;
            }
            Ok(())
        }
    }
    impl Drop for Mapping {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }
    struct Isolated;
    impl Environment for Isolated {
        fn read(&self, key: &str) -> anyhow::Result<Option<RegValue>> {
            UserEnvironment.read(key)
        }
        fn write(&self, key: &str, value: Option<&RegValue>) -> anyhow::Result<()> {
            UserEnvironment.write(key, value)
        }
        fn flush(&self) -> anyhow::Result<()> {
            UserEnvironment.flush()
        }
        fn acquire(&self) -> anyhow::Result<Box<dyn std::any::Any>> {
            UserEnvironment.acquire()
        }
        fn notify(&self) -> String {
            "Synthetic redirected fixture only; no environment broadcast sent.".into()
        }
    }
    let parent = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Pass an absolute scratch parent")?,
    );
    ensure!(parent.is_absolute(), "Scratch parent must be absolute");
    fs::create_dir_all(&parent)?;
    let id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let root = parent.join(format!("user-environment-probe-{id}"));
    fs::create_dir(&root)?;
    let actual_before = (
        UserEnvironment.read("Path")?,
        UserEnvironment.read("JAVA_HOME")?,
        UserEnvironment.read("CARGO_HOME")?,
    );
    let fixture_path = format!("Software\\SetupScopeEnvironmentProbe-{id}");
    ensure!(
        fixture_path.starts_with("Software\\SetupScopeEnvironmentProbe-")
            && id.bytes().all(|b| b.is_ascii_digit() || b == b'-'),
        "Unsafe synthetic key name"
    );
    let mut fixture = std::ptr::null_mut();
    let mut disposition = 0;
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(&fixture_path).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            std::ptr::null(),
            &mut fixture,
            &mut disposition,
        )
    };
    ensure!(rc == 0, "Cannot create isolated fixture key: {rc}");
    let fixture = Key(fixture);
    ensure!(
        disposition == REG_CREATED_NEW_KEY,
        "Fixture already exists; nothing will be overwritten"
    );
    let mut environment = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            fixture.0,
            wide("Environment").as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            std::ptr::null(),
            &mut environment,
            std::ptr::null_mut(),
        )
    };
    ensure!(rc == 0, "Cannot create fixture Environment: {rc}");
    let environment = Key(environment);
    let mut original = RegValue::string("pAtH", &format!("%USERPROFILE%\\SyntheticBefore-{id}"));
    original.kind = reg_type::EXPAND_SZ;
    let rc = unsafe {
        RegSetValueExW(
            environment.0,
            wide(&original.name).as_ptr(),
            0,
            original.kind,
            original.data.as_ptr(),
            original.data.len() as u32,
        )
    };
    ensure!(rc == 0, "Cannot seed isolated fixture: {rc}");
    drop(environment);
    let rc = unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, fixture.0) };
    ensure!(rc == 0, "Cannot redirect helper HKCU: {rc}");
    let mut mapping = Mapping(true);
    // Refuse all adapter writes unless the newly seeded fixture is observed.
    ensure!(
        UserEnvironment.read("Path")? == Some(original.clone()),
        "HKCU mapping did not expose the unique fixture; no adapter writes permitted"
    );
    let profile = Profile {
        version: 1,
        name: "Native isolated user environment fixture".into(),
        project: root.to_str().unwrap().into(),
        path_prepend: vec![root.join("sdk").to_str().unwrap().into()],
        variables: vec![Variable {
            name: "JAVA_HOME".into(),
            value: Some(root.join("java").to_str().unwrap().into()),
        }],
    };
    let prepared = envprofile::prepare_user(&profile, &Isolated)?;
    let applied = envprofile::apply_user(&root, &prepared, &Isolated)?;
    ensure!(
        UserEnvironment.read("JAVA_HOME")?.is_some(),
        "Native set was not observed"
    );
    ensure!(
        UserEnvironment
            .read("Path")?
            .as_ref()
            .is_some_and(|v| v.kind == reg_type::EXPAND_SZ && v.name == "pAtH"),
        "Native PATH type/case changed"
    );
    let loaded = envprofile::load_transaction(&root, &applied.id)?;
    envprofile::undo_user(&root, &loaded, &Isolated)?;
    ensure!(
        UserEnvironment.read("Path")? == Some(original)
            && UserEnvironment.read("JAVA_HOME")?.is_none(),
        "Native exact restore/absence mismatch"
    );
    mapping.restore()?;
    drop(fixture);
    ensure!(
        (
            UserEnvironment.read("Path")?,
            UserEnvironment.read("JAVA_HOME")?,
            UserEnvironment.read("CARGO_HOME")?
        ) == actual_before,
        "Real user Environment values changed"
    );
    // Remove only the exact unique key this helper successfully created.
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(&fixture_path).as_ptr()) };
    ensure!(
        rc == 0,
        "Synthetic fixture cleanup failed; inspect {fixture_path}: {rc}"
    );
    fs::write(root.join("native-result.txt"),"PASS: redirected native read/set/flush/delete/undo; exact PATH type/case/bytes and JAVA_HOME absence restored; real user Environment unchanged; synthetic key removed; no broadcast.\n")?;
    println!("PASS: native apply/flush/undo in process-local HKCU fixture; real user Environment unchanged; synthetic key removed.");
    println!("Evidence retained at {}", root.display());
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Native environment probe requires Windows");
    std::process::exit(1);
}
