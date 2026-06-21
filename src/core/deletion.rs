// deletion module
use std::path::Path;
use anyhow::Result;

pub fn move_to_trash<P: AsRef<Path>>(path: P) -> Result<()> {
    trash::delete(path)?;
    Ok(())
}

pub fn permanent_delete<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
