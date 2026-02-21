use std::path::PathBuf;

use directories::{BaseDirs, ProjectDirs};

fn project_dirs() -> ProjectDirs {
    ProjectDirs::from("", "", "nugs").expect("Could not determine platform directories")
}

pub fn config_dir() -> PathBuf {
    project_dirs().config_dir().to_path_buf()
}

pub fn cache_dir() -> PathBuf {
    project_dirs().cache_dir().to_path_buf()
}

/// Expand `~` prefix to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(base) = BaseDirs::new() {
            return base.home_dir().join(rest);
        }
    }
    PathBuf::from(path)
}
