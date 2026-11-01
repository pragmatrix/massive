use std::path::{Path, PathBuf};

/// The path of the desktop socket inside the desktop's projects directory, usually
/// `~/.massive/run/desktop.sock`. `$TMPDIR` is not used, because macOS purges files there that go
/// unused for a few days, and the desktop runs for long periods.
pub fn desktop_socket_path(projects_dir: &Path) -> PathBuf {
    projects_dir.join("run").join("desktop.sock")
}
