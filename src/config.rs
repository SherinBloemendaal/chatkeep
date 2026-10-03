//! Platform-specific configuration and paths

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Get the Cursor projects directory (~/.cursor/projects/)
pub fn cursor_projects_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".cursor").join("projects"))
}

/// Directory that holds Electron user data directories
/// - macOS: ~/Library/Application Support/
/// - Linux: ~/.config/
/// - Windows: %APPDATA%/
pub fn app_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join("Library").join("Application Support"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        dirs::config_dir().context("Could not determine config directory")
    }
}

/// Get the default Cursor user data directory
/// - macOS: ~/Library/Application Support/Cursor/
/// - Linux: ~/.config/Cursor/
/// - Windows: %APPDATA%/Cursor/
pub fn cursor_config_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("Cursor"))
}

/// Get the Cursor workspace storage directory
/// - macOS: ~/Library/Application Support/Cursor/User/workspaceStorage/
/// - Linux: ~/.config/Cursor/User/workspaceStorage/
/// - Windows: %APPDATA%/Cursor/User/workspaceStorage/
pub fn workspace_storage_dir() -> Result<PathBuf> {
    Ok(cursor_config_dir()?.join("User").join("workspaceStorage"))
}

/// Get the Cursor global storage directory
/// - macOS: ~/Library/Application Support/Cursor/User/globalStorage/
/// - Linux: ~/.config/Cursor/User/globalStorage/
/// - Windows: %APPDATA%/Cursor/User/globalStorage/
pub fn global_storage_dir() -> Result<PathBuf> {
    Ok(cursor_config_dir()?.join("User").join("globalStorage"))
}

/// State folder of the tool's former name, crepath.
const LEGACY_HOME: &str = ".crepath";

/// Local chatkeep state: history, backups, and the index. `CHATKEEP_HOME` overrides it.
///
/// The default home first takes over the state of a crepath install, once.
pub fn chatkeep_home() -> Result<PathBuf> {
    let overridden = std::env::var_os("CHATKEEP_HOME").filter(|path| !path.is_empty());
    let custom = overridden.is_some();
    let home = chatkeep_home_from(overridden)?;
    if !custom {
        let legacy = dirs::home_dir()
            .context("Could not determine home directory")?
            .join(LEGACY_HOME);
        adopt_legacy_state(&legacy, &home)?;
    }
    Ok(home)
}

/// Move everything except `bin` from a crepath state folder into a chatkeep home that does not
/// exist yet. The old binary stays in `bin` so `crepath uninstall` can still remove it.
///
/// Entries are gathered in a sibling staging folder that is renamed into place at the end, so an
/// interrupted run resumes instead of leaving a half-filled home behind.
pub fn adopt_legacy_state(legacy: &Path, home: &Path) -> Result<bool> {
    if home.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    let mut staging = home.as_os_str().to_owned();
    staging.push(".adopting");
    let staging = PathBuf::from(staging);
    fs::create_dir_all(&staging)
        .with_context(|| format!("failed to create {}", staging.display()))?;
    for entry in
        fs::read_dir(legacy).with_context(|| format!("failed to read {}", legacy.display()))?
    {
        let entry = entry?;
        if entry.file_name() == "bin" {
            continue;
        }
        let target = staging.join(entry.file_name());
        fs::rename(entry.path(), &target).with_context(|| {
            format!(
                "failed to move {} to {}",
                entry.path().display(),
                target.display()
            )
        })?;
    }
    fs::rename(&staging, home)
        .with_context(|| format!("failed to move {} to {}", staging.display(), home.display()))?;
    Ok(true)
}

pub fn chatkeep_home_from(overridden: Option<std::ffi::OsString>) -> Result<PathBuf> {
    if let Some(path) = overridden.filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".chatkeep"))
}

/// Get Cursor cache directories used by the Electron shell.
///
/// These are used for clearing stale in-memory/cache state after copy
/// operations to force index refresh in the running editor host.
pub fn cursor_cache_dirs() -> Result<Vec<PathBuf>> {
    let base = cursor_config_dir()?;
    Ok(vec![base.join("CachedData"), base.join("GPUCache")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_exist() {
        // These should not panic
        let _ = cursor_projects_dir();
        let _ = workspace_storage_dir();
        let _ = global_storage_dir();
    }

    #[test]
    fn test_cursor_projects_dir_structure() {
        let path = cursor_projects_dir().unwrap();
        // Should end with .cursor/projects
        let components: Vec<_> = path.components().collect();
        let len = components.len();
        assert!(len >= 2);
        assert_eq!(
            components[len - 1].as_os_str().to_string_lossy(),
            "projects"
        );
        assert_eq!(components[len - 2].as_os_str().to_string_lossy(), ".cursor");
    }

    #[test]
    fn test_workspace_storage_dir_structure() {
        let path = workspace_storage_dir().unwrap();
        // Should end with User/workspaceStorage
        let components: Vec<_> = path.components().collect();
        let len = components.len();
        assert!(len >= 2);
        assert_eq!(
            components[len - 1].as_os_str().to_string_lossy(),
            "workspaceStorage"
        );
        assert_eq!(components[len - 2].as_os_str().to_string_lossy(), "User");
    }

    #[test]
    fn test_global_storage_dir_structure() {
        let path = global_storage_dir().unwrap();
        // Should end with User/globalStorage
        let components: Vec<_> = path.components().collect();
        let len = components.len();
        assert!(len >= 2);
        assert_eq!(
            components[len - 1].as_os_str().to_string_lossy(),
            "globalStorage"
        );
        assert_eq!(components[len - 2].as_os_str().to_string_lossy(), "User");
    }

    #[test]
    fn legacy_state_moves_into_a_new_home_and_leaves_the_binary() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(".crepath");
        let home = root.path().join(".chatkeep");
        fs::create_dir_all(legacy.join("bin")).unwrap();
        fs::write(legacy.join("bin").join("crepath"), "binary").unwrap();
        fs::write(legacy.join("queue.jsonl"), "queue").unwrap();
        fs::create_dir_all(legacy.join("backups")).unwrap();
        fs::write(legacy.join("backups").join("one"), "backup").unwrap();

        assert!(adopt_legacy_state(&legacy, &home).unwrap());

        assert_eq!(
            fs::read_to_string(home.join("queue.jsonl")).unwrap(),
            "queue"
        );
        assert_eq!(
            fs::read_to_string(home.join("backups").join("one")).unwrap(),
            "backup"
        );
        assert!(!home.join("bin").exists());
        assert!(legacy.join("bin").join("crepath").exists());
        assert!(!legacy.join("queue.jsonl").exists());
        assert!(!root.path().join(".chatkeep.adopting").exists());
    }

    #[test]
    fn legacy_state_is_left_alone_once_the_home_exists() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(".crepath");
        let home = root.path().join(".chatkeep");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("queue.jsonl"), "old").unwrap();
        fs::create_dir_all(&home).unwrap();

        assert!(!adopt_legacy_state(&legacy, &home).unwrap());
        assert!(legacy.join("queue.jsonl").exists());
        assert!(!home.join("queue.jsonl").exists());
    }

    #[test]
    fn interrupted_adoption_resumes() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(".crepath");
        let home = root.path().join(".chatkeep");
        let staging = root.path().join(".chatkeep.adopting");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("history.jsonl"), "history").unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("queue.jsonl"), "queue").unwrap();

        assert!(adopt_legacy_state(&legacy, &home).unwrap());
        assert_eq!(
            fs::read_to_string(home.join("queue.jsonl")).unwrap(),
            "queue"
        );
        assert_eq!(
            fs::read_to_string(home.join("history.jsonl")).unwrap(),
            "history"
        );
    }

    #[test]
    fn missing_legacy_state_is_a_no_op() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join(".chatkeep");
        assert!(!adopt_legacy_state(&root.path().join(".crepath"), &home).unwrap());
        assert!(!home.exists());
    }

    #[test]
    fn chatkeep_home_can_be_overridden() {
        assert_eq!(
            chatkeep_home_from(Some("/tmp/state".into())).unwrap(),
            PathBuf::from("/tmp/state")
        );
        let default = chatkeep_home_from(None).unwrap();
        assert!(default.ends_with(".chatkeep"));
        assert_eq!(chatkeep_home_from(Some("".into())).unwrap(), default);
    }

    #[test]
    fn test_paths_share_common_base() {
        // workspace_storage_dir and global_storage_dir should share base up to User/
        let ws = workspace_storage_dir().unwrap();
        let gs = global_storage_dir().unwrap();

        let ws_parent = ws.parent().unwrap();
        let gs_parent = gs.parent().unwrap();

        assert_eq!(ws_parent, gs_parent);
    }
}
