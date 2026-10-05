//! The background runner of `chatkeep claude sync watch`: a per-user launch agent on macOS.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::ui;

pub const LABEL: &str = "dev.chatkeep.sync";

/// Where the watcher writes what it did.
pub fn log_path(home: &Path) -> PathBuf {
    home.join("sync.log")
}

pub fn agent_path(user_home: &Path) -> PathBuf {
    user_home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launch agent: start the watcher at login and keep it running. `env` carries the
/// settings chatkeep was run with, so the agent uses the same folders.
pub fn agent_text(exe: &Path, env: &[(String, String)]) -> String {
    let mut vars = String::new();
    if !env.is_empty() {
        vars.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (key, value) in env {
            vars.push_str(&format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                xml(key),
                xml(value)
            ));
        }
        vars.push_str("  </dict>\n");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>claude</string>
    <string>sync</string>
    <string>watch</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ProcessType</key>
  <string>Background</string>
{vars}</dict>
</plist>
"#,
        xml(&exe.to_string_lossy())
    )
}

/// The settings a background watcher has to inherit.
pub fn inherited_env() -> Vec<(String, String)> {
    ["CHATKEEP_HOME", "CLAUDE_CONFIG_DIR", "CHATKEEP_NO_INDEX"]
        .into_iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (key.to_string(), value))
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use anyhow::{Context, bail};
    use std::os::unix::fs::MetadataExt;
    use std::process::Command;

    fn user_home() -> Result<PathBuf> {
        dirs::home_dir().context("could not determine home directory")
    }

    fn domain() -> Result<String> {
        let uid = std::fs::metadata(user_home()?)?.uid();
        Ok(format!("gui/{uid}"))
    }

    fn launchctl(args: &[&str]) -> Result<std::process::Output> {
        Command::new("launchctl")
            .args(args)
            .output()
            .context("failed to run launchctl")
    }

    pub fn running() -> Result<bool> {
        Ok(launchctl(&["print", &format!("{}/{LABEL}", domain()?)])?
            .status
            .success())
    }

    pub fn installed() -> Result<bool> {
        Ok(agent_path(&user_home()?).is_file())
    }

    pub fn enable() -> Result<PathBuf> {
        let exe = std::env::current_exe().context("cannot find the chatkeep binary")?;
        let path = agent_path(&user_home()?);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let domain = domain()?;
        // Replace a runner from an earlier install, whatever binary it pointed at.
        launchctl(&["bootout", &format!("{domain}/{LABEL}")])?;
        std::fs::write(&path, agent_text(&exe, &inherited_env()))
            .with_context(|| format!("failed to write {}", path.display()))?;
        let started = launchctl(&["bootstrap", &domain, &path.to_string_lossy()])?;
        if !started.status.success() {
            bail!(
                "launchctl could not start the background sync: {}",
                String::from_utf8_lossy(&started.stderr).trim()
            );
        }
        Ok(path)
    }

    /// Stop and remove the runner. Returns whether there was one.
    pub fn disable() -> Result<bool> {
        let path = agent_path(&user_home()?);
        let was_running = running()?;
        if was_running {
            launchctl(&["bootout", &format!("{}/{LABEL}", domain()?)])?;
        }
        let existed = path.is_file();
        if existed {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(existed || was_running)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    fn unsupported() -> anyhow::Error {
        ui::hinted(
            "the background sync is only built for macOS so far",
            "Start chatkeep claude sync watch yourself when you log in.",
        )
    }

    pub fn running() -> Result<bool> {
        Ok(false)
    }

    pub fn installed() -> Result<bool> {
        Ok(false)
    }

    pub fn enable() -> Result<PathBuf> {
        Err(unsupported())
    }

    pub fn disable() -> Result<bool> {
        Ok(false)
    }
}

pub use platform::{disable, enable, installed, running};

/// One line on what the runner is doing.
pub fn status_line(home: &Path) -> Result<String> {
    let state = match (installed()?, running()?) {
        (true, true) => "on: syncing in the background",
        (true, false) => {
            "installed, but not running; turn it on again with chatkeep claude sync auto on"
        }
        (false, _) => "off",
    };
    Ok(format!(
        "background sync is {state} (log: {})",
        ui::home_relative(&log_path(home).display().to_string())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_starts_the_watcher_and_keeps_its_settings() {
        let text = agent_text(
            Path::new("/opt/a&b/chatkeep"),
            &[("CHATKEEP_HOME".to_string(), "/state/<x>".to_string())],
        );
        assert!(text.contains("<string>/opt/a&amp;b/chatkeep</string>"));
        assert!(text.contains(
            "<string>claude</string>\n    <string>sync</string>\n    <string>watch</string>"
        ));
        assert!(text.contains("<key>CHATKEEP_HOME</key>\n    <string>/state/&lt;x&gt;</string>"));
        assert!(text.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(!agent_text(Path::new("/x"), &[]).contains("EnvironmentVariables"));
        assert!(
            agent_path(Path::new("/Users/me"))
                .ends_with("Library/LaunchAgents/dev.chatkeep.sync.plist")
        );
    }
}
