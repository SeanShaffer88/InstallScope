//! Explicit disk-backed scratch folders. No global environment, disk, or cleanup changes.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn create(parent: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        parent.is_absolute(),
        "Choose an absolute existing local parent folder"
    );
    match crate::devcheck::probe(parent) {
        crate::devcheck::Probe::Dir => {},
        other => anyhow::bail!("Choose an existing local directory without links or reparse points: {other:?}"),
    }
    let parent = parent.canonicalize()?;
    anyhow::ensure!(parent.is_dir(), "Scratch parent must be a directory");
    let folder = parent.join(format!("SetupScope-Scratch-{}", crate::util::new_id()));
    fs::create_dir(&folder)?; // Never reuse or overwrite an existing workspace.
    let result = (|| -> anyhow::Result<()> {
        let mut note = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join("SCRATCH-WORKSPACE.txt"))?;
        note.write_all(b"SetupScope disk-backed scratch workspace.\r\nNot a RAM disk or sandbox. No automatic deletion or quota.\r\nKeep durable source and backups elsewhere. Review and remove manually when finished.\r\n")?;
        note.sync_all()?;
        Ok(())
    })();
    result.map_err(|e| {
        anyhow::anyhow!(
            "Workspace exists at {} but its notice could not be written: {e}",
            folder.display()
        )
    })?;
    Ok(folder)
}

/// Literal PowerShell assignments, copied only; never executed by this module.
pub fn powershell_environment(folder: &Path) -> String {
    let literal = folder.to_string_lossy().replace('\'', "''");
    format!("$env:TEMP = '{literal}'\n$env:TMP = '{literal}'\n# Applies only to this shell and child processes. Restore by closing this shell.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_is_unique_and_preserves_existing_content() {
        let parent = std::env::temp_dir().join(format!("scratch-test-{}", crate::util::new_id()));
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("keep.txt"), b"keep").unwrap();
        let a = create(&parent).unwrap();
        let b = create(&parent).unwrap();
        assert_ne!(a, b);
        assert!(a.join("SCRATCH-WORKSPACE.txt").is_file());
        assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"keep");
        assert!(create(Path::new("relative")).is_err());
        assert!(create(&parent.join("keep.txt")).is_err());
        fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn shell_literal_does_not_expand_input() {
        let text = powershell_environment(Path::new("C:/dev/it's $special;folder"));
        assert!(text.contains("'C:/dev/it''s $special;folder'"));
    }
}
