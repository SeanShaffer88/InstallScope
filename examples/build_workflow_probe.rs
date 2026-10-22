//! Isolated real Rust build/test through the reviewed runner; never installs tools.
use setupscope_core::{build_run::{self,Request},snapshot::Progress};
use anyhow::{ensure,Result};
fn main()->Result<()> {
    let temp=std::env::temp_dir().join(format!("SetupScope-workflow-{}",setupscope_core::util::new_id()));
    std::fs::create_dir(&temp)?;
    struct Fixture(std::path::PathBuf);
    impl Fixture{fn path(&self)->&std::path::Path{&self.0}}
    impl Drop for Fixture{fn drop(&mut self){if let(Ok(root),Ok(parent))=(std::env::temp_dir().canonicalize(),self.0.parent().unwrap().canonicalize()){if root==parent&&self.0.file_name().unwrap().to_string_lossy().starts_with("SetupScope-workflow-"){let _=std::fs::remove_dir_all(&self.0);}}}}
    let temp=Fixture(temp);
    let cargo=std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).ok_or_else(||anyhow::anyhow!("USERPROFILE missing"))?.join(".cargo/bin/cargo.exe");
    std::fs::write(temp.path().join("Cargo.toml"),"[package]\nname=\"setupscope-workflow-fixture\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[lib]\npath=\"lib.rs\"\n")?;
    std::fs::write(temp.path().join("lib.rs"),"pub fn add(a:u32,b:u32)->u32{a+b}\n#[test]fn sum(){assert_eq!(add(2,3),5);}\n")?;
    let p=Progress::default();
    for arguments in [vec!["build","--offline"],vec!["test","--offline"]] {
        let request=Request{project:temp.path().into(),executable:cargo.clone(),arguments:arguments.iter().map(|s|s.to_string()).collect(),logs:temp.path().into(),timeout_seconds:120};
        let out=build_run::run(build_run::review(request)?,true,&p)?;
        println!("{}",out.summary);ensure!(out.summary.contains("Completed; exit 0"),"Fixture build/test failed");
        let _=setupscope_core::build_log::inspect(&out.log,&p)?;
    }
    println!("Real isolated offline Rust build and one test passed; logs were triage-readable. Temporary fixture is removed on exit.");Ok(())
}
