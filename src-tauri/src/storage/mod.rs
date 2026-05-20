pub mod artifacts;
pub mod changelog;
pub mod claude_usage;
pub mod cli_config;
pub mod cli_sessions;
pub mod community_skills;
pub mod events;
pub mod favorites;
pub mod helion_usage;
pub mod mcp_registry;
pub mod plugins;
pub mod prompt_index;
pub mod run_index;
pub mod runs;
pub mod settings;
pub mod teams;

use std::path::{Path, PathBuf};

const DATA_DIR_NAME: &str = ".helioncoder";
const LEGACY_DATA_DIR_NAME: &str = ".opencovibe";

pub fn data_dir() -> PathBuf {
    let home = dirs_next().expect("Could not determine home directory");
    let dir = home.join(DATA_DIR_NAME);
    migrate_legacy_data_dir(&home, &dir);
    dir
}

fn migrate_legacy_data_dir(home: &Path, dir: &Path) {
    let legacy = home.join(LEGACY_DATA_DIR_NAME);
    if !legacy.exists() {
        return;
    }

    if !legacy.is_dir() {
        log::warn!(
            "[storage] legacy data path exists but is not a directory: {}",
            legacy.display()
        );
        return;
    }

    if !dir.exists() {
        match std::fs::rename(&legacy, dir) {
            Ok(()) => {
                log::info!(
                    "[storage] migrated legacy data directory {} to {}",
                    legacy.display(),
                    dir.display()
                );
                return;
            }
            Err(e) => {
                log::warn!(
                    "[storage] failed to rename legacy data directory {} to {}: {}",
                    legacy.display(),
                    dir.display(),
                    e
                );
            }
        }
    }

    if dir.exists() && !dir.is_dir() {
        log::warn!(
            "[storage] target data path exists but is not a directory: {}",
            dir.display()
        );
        return;
    }

    if let Err(e) = merge_legacy_data_dir(&legacy, dir) {
        log::warn!(
            "[storage] failed to merge legacy data directory {} into {}: {}",
            legacy.display(),
            dir.display(),
            e
        );
    }
}

fn merge_legacy_data_dir(legacy: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;

    for entry in std::fs::read_dir(legacy)? {
        let entry = entry?;
        let source = entry.path();
        let dest = dir.join(entry.file_name());
        let dest = if dest.exists() {
            next_legacy_backup_path(&dest)
        } else {
            dest
        };
        std::fs::rename(&source, &dest)?;
    }

    std::fs::remove_dir(legacy)?;
    Ok(())
}

fn next_legacy_backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("legacy");

    let mut candidate = parent.join(format!("{}.legacy-backup", file_name));
    let mut index = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{}.legacy-backup-{}", file_name, index));
        index += 1;
    }
    candidate
}

pub fn runs_dir() -> PathBuf {
    data_dir().join("runs")
}

pub fn run_dir(run_id: &str) -> PathBuf {
    runs_dir().join(run_id)
}

/// Resolve the user's home directory reliably.
/// Primary: `getpwuid()` system call (works even when `$HOME` is unset,
/// e.g. GUI apps launched from Finder/Dock on macOS 26+).
/// Fallback: `$HOME` (Unix) or `$USERPROFILE` (Windows).
pub fn home_dir() -> Option<String> {
    #[cfg(unix)]
    {
        let pwd_home = unsafe {
            let uid = libc::getuid();
            let pw = libc::getpwuid(uid);
            if !pw.is_null() {
                let dir = (*pw).pw_dir;
                if !dir.is_null() {
                    Some(std::ffi::CStr::from_ptr(dir).to_string_lossy().into_owned())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if pwd_home.is_some() {
            return pwd_home;
        }
        std::env::var("HOME").ok()
    }
    #[cfg(not(unix))]
    {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .or_else(|_| {
                let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
                let path = std::env::var("HOMEPATH").unwrap_or_default();
                if !drive.is_empty() && !path.is_empty() {
                    Ok(format!("{}{}", drive, path))
                } else {
                    Err(std::env::VarError::NotPresent)
                }
            })
            .ok()
    }
}

pub(crate) fn dirs_next() -> Option<PathBuf> {
    home_dir().map(PathBuf::from)
}

pub fn ensure_dir(path: &std::path::Path) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }

    // Restrict directory permissions — data dir may contain sensitive data
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_data_dir_when_target_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let legacy = home.join(LEGACY_DATA_DIR_NAME);
        let target = home.join(DATA_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("settings.json"), "{}").unwrap();

        migrate_legacy_data_dir(home, &target);

        assert!(target.join("settings.json").is_file());
        assert!(!legacy.exists());
    }

    #[test]
    fn merges_legacy_data_dir_without_overwriting_target_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let legacy = home.join(LEGACY_DATA_DIR_NAME);
        let target = home.join(DATA_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(legacy.join("settings.json"), "legacy").unwrap();
        std::fs::write(legacy.join("prompt-favorites.json"), "favorites").unwrap();
        std::fs::write(target.join("settings.json"), "current").unwrap();

        migrate_legacy_data_dir(home, &target);

        assert_eq!(
            std::fs::read_to_string(target.join("settings.json")).unwrap(),
            "current"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("settings.json.legacy-backup")).unwrap(),
            "legacy"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("prompt-favorites.json")).unwrap(),
            "favorites"
        );
        assert!(!legacy.exists());
    }
}
