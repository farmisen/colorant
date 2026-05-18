//! Filesystem helpers shared across the binary.
//!
//! Right now it's just `atomic_write` — staging to `<dest>.tmp` and
//! renaming over. Used both by `gogh::sync` (catalog index) and by the
//! TUI's two-phase apply (per-palette writes) so a disk-full or signal
//! in the middle never leaves a half-written file behind.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Write `content` to `dest` atomically: stages into `<dest>.tmp` and
/// renames over. Avoids leaving a half-written file when the write is
/// interrupted (disk full, signal, etc.).
pub fn atomic_write(dest: &Path, content: &str) -> Result<()> {
    let tmp = dest.with_extension(match dest.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".to_string(),
    });
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, dest)
        .with_context(|| format!("renaming {} to {}", tmp.display(), dest.display()))?;
    Ok(())
}
