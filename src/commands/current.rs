//! `colorant current` — print the path of the .colorantrc that would be
//! applied for the current directory (or nothing, exit 0, if none).

use crate::config::THEME_FILE_NAME;
use crate::walk;
use anyhow::Result;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    if let Some(path) = walk::find_nearest(&cwd, THEME_FILE_NAME) {
        println!("{}", path.display());
    }
    Ok(())
}
