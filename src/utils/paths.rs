use std::path::{Path, PathBuf};

pub fn expand_tilde<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref();
    if !p.starts_with("~") {
        return p.to_path_buf();
    }
    
    if p == Path::new("~") {
        return dirs::home_dir().unwrap_or_else(|| p.to_path_buf());
    }

    if let Ok(stripped) = p.strip_prefix("~/") {
        if let Some(mut home) = dirs::home_dir() {
            home.push(stripped);
            return home;
        }
    }

    p.to_path_buf()
}
