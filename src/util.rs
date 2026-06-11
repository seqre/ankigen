//! Small shared helpers.

use std::path::{Path, PathBuf};

/// Write `content` to `path` atomically (write a sibling temp file, then
/// rename over the target) so a crash mid-write can never corrupt the original.
pub fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".ankigen-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
