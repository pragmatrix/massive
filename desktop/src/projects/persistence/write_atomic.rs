//! The atomic write of the configuration file, which keeps a symlinked file a symlink.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Writes `text` to `path` atomically, through a temp file in the target's directory that is
/// renamed over the target.
///
/// A symlinked `path`, such as one into a dotfiles repository, is resolved first, so the rename
/// replaces the file it points to and the link stays. A dangling symlink fails. An existing file
/// keeps its permissions, and a new one gets the temp file's 0600, because it can hold commands.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("resolving the symlink {}", path.display()))?,
        _ => path.to_path_buf(),
    };
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory).with_context(|| format!("creating {}", directory.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(text.as_bytes())?;
    if let Ok(metadata) = fs::metadata(&target) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.persist(&target)
        .map_err(|error| anyhow::anyhow!("persisting {}", error.error))?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    /// A write through a symlink replaces the file it points to, keeps the link and the file's
    /// permissions, and fails for a dangling link instead of replacing it.
    #[test]
    fn a_write_through_a_symlink_keeps_the_link_and_the_mode() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("dotfiles").join("desktop.kdl");
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(&target, "old")?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))?;
        let link = dir.path().join("desktop.kdl");
        symlink(&target, &link)?;

        write_atomic(&link, "new")?;

        assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target)?, "new");
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o644);

        let dangling = dir.path().join("dangling.kdl");
        symlink(dir.path().join("missing.kdl"), &dangling)?;
        assert!(write_atomic(&dangling, "text").is_err());
        assert!(fs::symlink_metadata(&dangling)?.file_type().is_symlink());
        Ok(())
    }
}
