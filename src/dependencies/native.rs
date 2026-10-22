//! Microsoft Build Tools vendor-managed workloads/layouts. Never treated as a pinned full installer bundle.
use super::*;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Install,
    CreateLayout,
    VerifyLayout,
    InstallOffline,
}
impl Action {
    pub const ALL: [Self; 4] = [
        Self::Install,
        Self::CreateLayout,
        Self::VerifyLayout,
        Self::InstallOffline,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "Install Selected Components",
            Self::CreateLayout => "Create / Update Offline Layout",
            Self::VerifyLayout => "Verify Offline Layout",
            Self::InstallOffline => "Install From Offline Layout",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub bootstrapper: PathBuf,
    pub layout: PathBuf,
    pub action: Action,
    pub arm64: bool,
    pub recommended: bool,
    pub cmake: bool,
    pub sdk: bool,
    pub language: String,
    pub elevated: bool,
}
pub struct Review {
    install: InstallReview,
    package: Package,
    arguments: String,
    options: Options,
}
impl Review {
    pub fn text(&self) -> &str {
        self.install.text()
    }
}
pub fn arguments(o: &Options) -> Result<String> {
    ensure!(
        matches!(
            o.language.as_str(),
            "en-US" | "de-DE" | "es-ES" | "fr-FR" | "ja-JP" | "zh-CN"
        ),
        "Choose a supported layout language"
    );
    let mut args = Vec::new();
    if o.action != Action::Install {
        local_path(&o.layout)?;
        ensure!(!o.layout.as_os_str().is_empty(), "Choose a layout folder");
    }
    match o.action {
        Action::CreateLayout => {
            args.extend([
                "--layout".into(),
                crate::build_run::quote(&o.layout.to_string_lossy()),
            ]);
        }
        Action::VerifyLayout => {
            return Ok(format!(
                "--layout {} --verify",
                crate::build_run::quote(&o.layout.to_string_lossy())
            ))
        }
        Action::InstallOffline => {
            ensure!(o.layout.is_dir(), "Layout folder must exist");
            ensure!(
                o.bootstrapper.parent() == Some(o.layout.as_path()),
                "Select vs_buildtools.exe from the offline layout root"
            );
            args.push("--noWeb".into());
        }
        Action::Install => {}
    }
    args.extend([
        "--add".into(),
        "Microsoft.VisualStudio.Workload.VCTools".into(),
        "--add".into(),
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64".into(),
    ]);
    if o.recommended {
        args.push("--includeRecommended".into());
    }
    if o.arm64 {
        args.extend([
            "--add".into(),
            "Microsoft.VisualStudio.Component.VC.Tools.ARM64".into(),
        ]);
    }
    if o.cmake {
        args.extend([
            "--add".into(),
            "Microsoft.VisualStudio.Component.VC.CMake.Project".into(),
        ]);
    }
    if o.sdk {
        args.extend([
            "--add".into(),
            "Microsoft.VisualStudio.Component.Windows11SDK.26100".into(),
        ]);
    }
    if o.action == Action::CreateLayout {
        args.extend(["--lang".into(), o.language.clone()]);
    } else {
        args.extend(["--wait".into(), "--norestart".into()]);
    }
    Ok(args.join(" "))
}
pub fn review(o: Options, progress: &Progress) -> Result<Review> {
    checkpoint(progress)?;
    local_path(&o.bootstrapper)?;
    ensure!(
        o.bootstrapper
            .file_name()
            .is_some_and(|s| s.eq_ignore_ascii_case("vs_buildtools.exe")),
        "Choose Microsoft's vs_buildtools.exe bootstrapper"
    );
    let arguments = arguments(&o)?;
    let mut held = lock(&o.bootstrapper)?;
    let bytes = held.file.metadata()?.len();
    ensure!(
        bytes > 0 && bytes <= 64 * 1024 * 1024,
        "Bootstrapper must be 1 byte–64 MiB"
    );
    let hash = digest(&mut held.file, "SHA256", progress)?;
    let p = Package {
        id: "native-build-tools".into(),
        name: "Microsoft Visual Studio Build Tools".into(),
        version: "Vendor-selected channel; not pinned".into(),
        arch: if o.arm64 {
            "x64 + ARM64 targets"
        } else {
            "x64/x86 targets"
        }
        .into(),
        filename: "vs_buildtools.exe".into(),
        url: String::new(),
        bytes,
        algorithm: "SHA256".into(),
        digest: hash,
        publisher: "Microsoft Corporation".into(),
        kind: "exe".into(),
        source: "https://learn.microsoft.com/en-us/visualstudio/releases/2022/release-history"
            .into(),
        license_url: "https://visualstudio.microsoft.com/license-terms/".into(),
        notes: String::new(),
    };
    let signer = signature(&p, &o.bootstrapper)?;
    let command = format!("\"{}\" {arguments}", o.bootstrapper.display());
    let text=format!("MICROSOFT NATIVE TOOLCHAIN — {}\n{}\nSHA256: {}\nSignature: {}\nElevation requested: {}\n\nChoose a Build Tools 2022 bootstrapper from Microsoft's release-history page. Its hash is observed locally, NOT pinned to upstream metadata. The Microsoft signature is checked using cached Windows trust without live revocation. A valid publisher does not establish the product, channel, component availability or license eligibility; review file properties and vendor UI.\n\nMSVC x64/x86 is selected. ARM64 target tools: {}. Windows SDK 26100: {}. CMake/Ninja component: {}. Recommended components: {}. Component IDs are verified for Build Tools 2022; other versions require vendor confirmation. Workload dependencies and channel updates are vendor-controlled; this is not exact reproducibility. No silent installation or automatic license acceptance.\n\nInstall changes Windows and may need UAC/reboot. Creating/updating a layout downloads several GiB, can overwrite existing layout content and may install/update the shared installer; SetupScope does not estimate total size or certify layout completeness. Verify uses the vendor's checker; offline install uses --noWeb, but is not a network sandbox and may still need prerequisite certificates/installer updates. No automatic certificate import. Vendor UI controls cancellation. SetupScope tracks only the launched process; detached work may continue. Recovery: Visual Studio Installer Modify/Repair/Uninstall; no automatic rollback. Keep your own backup of a layout before updating. Pinned .NET/CMake bundle import/export does NOT include this layout.\n",o.action.label(),command,p.digest,signer,o.elevated,o.arm64,o.sdk,o.cmake,o.recommended);
    Ok(Review {
        install: InstallReview {
            id: p.id.clone(),
            path: o.bootstrapper.clone(),
            elevated: o.elevated,
            text,
            _held: held,
            command: Some(command),
        },
        package: p,
        arguments,
        options: o,
    })
}
pub fn launch(
    root: &Path,
    review: Review,
    acknowledged: bool,
    progress: &Progress,
) -> Result<String> {
    let _operation = vendor_operation()?;
    ensure!(
        arguments(&review.options)? == review.arguments,
        "Native operation inputs changed; prepare a fresh review"
    );
    #[cfg(windows)]
    {
        let arguments = review.arguments;
        install_with(
            root,
            review.install,
            acknowledged,
            progress,
            review.package,
            signature,
            move |_, path, elevated| {
                windows::launch_command(
                    path,
                    &arguments,
                    elevated,
                    path.parent().context("Bootstrapper parent missing")?,
                )
            },
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (root, review, acknowledged, progress);
        bail!("Native setup requires Windows")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            bootstrapper: PathBuf::from("C:/Tools/vs_buildtools.exe"),
            layout: std::env::temp_dir().join("SetupScope-VSLayout-test"),
            action: Action::Install,
            arm64: false,
            recommended: false,
            cmake: true,
            sdk: true,
            language: "en-US".into(),
            elevated: false,
        }
    }
    #[test]
    fn minimum_and_optional_components_are_explicit() {
        let mut o = options();
        let a = arguments(&o).unwrap();
        assert!(
            a.contains("VCTools")
                && a.contains("Windows11SDK.26100")
                && a.contains("VC.CMake.Project")
                && !a.contains("--quiet")
        );
        assert!(!a.contains("--includeRecommended"));
        o.arm64 = true;
        assert!(arguments(&o).unwrap().contains("VC.Tools.ARM64"));
    }
    #[test]
    fn layout_verification_never_adds_install_components() {
        let mut o = options();
        o.action = Action::VerifyLayout;
        let a = arguments(&o).unwrap();
        assert!(a.ends_with("--verify"));
        assert!(!a.contains("--add"));
    }
    #[test]
    fn foreign_language_and_layout_traversal_blocked() {
        let mut o = options();
        o.language = "en-US --quiet".into();
        assert!(arguments(&o).is_err());
        o.language = "en-US".into();
        o.action = Action::CreateLayout;
        o.layout = "C:/layout/../elsewhere".into();
        assert!(arguments(&o).is_err());
    }
}
