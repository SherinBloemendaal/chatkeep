//! Remove the managed chatkeep install and the PATH entry the installer added.

use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Color};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::UninstallArgs;
use crate::engine::index::{self, Holder};
use crate::ui::{self, Theme};

pub fn run(args: &UninstallArgs) -> Result<()> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let binary = crate::update::install_binary_path()?;
    let state = crate::config::chatkeep_home()?;
    let spellings = dir_spellings(binary.parent().unwrap_or(Path::new("")));
    execute(args, &home, &binary, &state, &spellings)?;
    // The background sync runs this binary; without it, it would restart forever.
    if args.dry_run {
        if crate::claude::autosync::installed()? {
            ui::info("The background sync would be stopped and removed.");
        }
    } else if crate::claude::autosync::disable()? {
        ui::ok("Stopped and removed the background sync.");
    }
    Ok(())
}

fn execute(
    args: &UninstallArgs,
    home: &Path,
    binary: &Path,
    state: &Path,
    spellings: &[String],
) -> Result<()> {
    let theme = Theme::stdout();
    let outside = outside_note(binary);
    let edits = path_edits(home, spellings)?;
    let links = managed_links(&link_dirs(home), binary, spellings);
    let purge_block = if args.purge {
        live_refresh(state)
    } else {
        None
    };
    if let Some(note) = &outside {
        ui::info(note);
    }
    let binary_label = if binary.exists() { "remove" } else { "absent" };
    let path_label = path_plan_label(&edits);
    let purge_label = if args.purge && state.exists() && purge_block.is_none() {
        "yes"
    } else {
        "no"
    };
    let link_label = link_plan_label(&links);
    if args.dry_run {
        ui::section("Uninstall (dry run)");
        print_table(theme, binary_label, &link_label, &path_label, purge_label);
        if let Some(holder) = &purge_block {
            ui::warn(&lock_message(holder));
        }
        println!(
            "{}",
            ui::hint_line(theme, "Nothing was changed. Run again without -n to apply.",)
        );
        return Ok(());
    }
    let work =
        binary.exists() || !edits.is_empty() || !links.is_empty() || (args.purge && state.exists());
    if work && !args.yes {
        ui::section("Uninstall");
        print_table(theme, binary_label, &link_label, &path_label, purge_label);
        if !ui::confirm("Remove the chatkeep install?", false, ui::interactive())? {
            bail!("aborted");
        }
    }
    let (unlinked, link_error) = remove_links(&links);
    let binary_result = remove_binary(binary)?;
    let edited = apply_path_edits(&edits)?;
    let (purge_result, purge_error) = if args.purge {
        match purge_state(state, purge_block) {
            Ok(yes) => (if yes { "yes" } else { "no" }, None),
            Err(err) => ("no", Some(err)),
        }
    } else {
        ("no", None)
    };
    ui::section("Uninstall");
    print_table(
        theme,
        binary_result.label,
        &linked_label(&unlinked, link_error.is_some()),
        &edited_label(&edited),
        purge_result,
    );
    if let Some(err) = binary_result.error {
        return Err(err);
    }
    if let Some(err) = link_error {
        return Err(err);
    }
    if let Some(err) = purge_error {
        return Err(err);
    }
    Ok(())
}

struct BinaryResult {
    label: &'static str,
    error: Option<anyhow::Error>,
}

fn remove_binary(binary: &Path) -> Result<BinaryResult> {
    if !binary.exists() {
        return Ok(BinaryResult {
            label: "absent",
            error: None,
        });
    }
    refuse_cursor(binary)?;
    match fs::remove_file(binary) {
        Ok(()) => Ok(BinaryResult {
            label: "removed",
            error: None,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BinaryResult {
            label: "absent",
            error: None,
        }),
        Err(err) => Ok(BinaryResult {
            label: "failed",
            error: Some(
                anyhow::Error::from(err).context(format!("could not remove {}", binary.display())),
            ),
        }),
    }
}

fn purge_state(state: &Path, blocked: Option<Holder>) -> Result<bool> {
    if !state.exists() {
        return Ok(false);
    }
    if let Some(holder) = blocked {
        bail!("{}", lock_message(&holder));
    }
    refuse_cursor(state)?;
    fs::remove_dir_all(state).with_context(|| format!("could not delete {}", state.display()))?;
    Ok(true)
}

fn live_refresh(state: &Path) -> Option<Holder> {
    index::holder(state).filter(|holder| holder.alive)
}

fn lock_message(holder: &Holder) -> String {
    if holder.pid == 0 {
        "a background chatkeep __refresh-index holds the index lock, so local state was not deleted"
            .to_string()
    } else {
        format!(
            "a background chatkeep __refresh-index holds the index lock (pid {}), so local state was not deleted",
            holder.pid
        )
    }
}

fn outside_note(binary: &Path) -> Option<String> {
    if !binary.is_file() {
        return None;
    }
    let current = std::env::current_exe().ok()?;
    if same_file(&current, binary) {
        return None;
    }
    Some(format!(
        "This chatkeep is {}, outside {}. The managed install will still be removed.",
        current.display(),
        binary.display()
    ))
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn print_table(theme: Theme, binary: &str, link: &str, path: &str, purge: &str) {
    let cell = |text: &str| {
        let good = matches!(text, "removed" | "yes")
            || text.starts_with("edited ")
            || text.starts_with("removed ");
        if text == "failed" || text.starts_with("failed ") {
            theme.cell(
                format!("{} {text}", theme.icons().cross),
                Some(Color::Red),
                &[Attribute::Bold],
            )
        } else if good {
            theme.cell(
                format!("{} {text}", theme.icons().check),
                Some(Color::Green),
                &[Attribute::Bold],
            )
        } else if text == "remove" || text.starts_with("remove ") || text.starts_with("edit ") {
            theme.cell(text, Some(Color::Yellow), &[Attribute::Bold])
        } else {
            theme.cell(text, None, &[Attribute::Dim])
        }
    };
    println!(
        "{}",
        ui::panel(
            theme,
            if cfg!(windows) {
                vec![
                    ("Binary", cell(binary)),
                    ("PATH", cell(path)),
                    ("Purge", cell(purge)),
                ]
            } else {
                vec![
                    ("Binary", cell(binary)),
                    ("Link", cell(link)),
                    ("PATH", cell(path)),
                    ("Purge", cell(purge)),
                ]
            },
        )
    );
}

/// Directories the installer may put a `chatkeep` symlink in, in its search order.
fn link_dirs(home: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    vec![
        home.join(".local").join("bin"),
        home.join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// `chatkeep` symlinks in `dirs` that point at the managed binary, even when it is already gone.
/// Regular files and links to anything else are never returned.
fn managed_links(dirs: &[PathBuf], binary: &Path, spellings: &[String]) -> Vec<PathBuf> {
    let name = binary.file_name().unwrap_or_default();
    let mut targets: Vec<PathBuf> = vec![binary.to_path_buf()];
    for spelling in spellings {
        let candidate = Path::new(spelling).join(name);
        if !targets.contains(&candidate) {
            targets.push(candidate);
        }
    }
    let canonical = fs::canonicalize(binary).ok();
    dirs.iter()
        .map(|dir| dir.join(name))
        .filter(|link| {
            let is_link = fs::symlink_metadata(link)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false);
            if !is_link {
                return false;
            }
            let Ok(raw) = fs::read_link(link) else {
                return false;
            };
            let resolved = lexical(&if raw.is_relative() {
                link.parent().unwrap_or(Path::new("")).join(&raw)
            } else {
                raw.clone()
            });
            targets.contains(&raw)
                || targets.contains(&resolved)
                || canonical.as_ref().is_some_and(|canonical| {
                    fs::canonicalize(link).ok().as_ref() == Some(canonical)
                })
        })
        .collect()
}

/// Resolve `.` and `..` without touching the filesystem, so dangling links still compare.
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(part);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn remove_links(links: &[PathBuf]) -> (Vec<PathBuf>, Option<anyhow::Error>) {
    let mut removed = Vec::new();
    for link in links {
        match fs::remove_file(link) {
            Ok(()) => removed.push(link.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return (
                    removed,
                    Some(
                        anyhow::Error::from(err)
                            .context(format!("could not remove {}", link.display())),
                    ),
                );
            }
        }
    }
    (removed, None)
}

fn shown_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| ui::home_relative(&path.display().to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn link_plan_label(links: &[PathBuf]) -> String {
    if links.is_empty() {
        "none".to_string()
    } else {
        format!("remove {}", shown_paths(links))
    }
}

fn linked_label(removed: &[PathBuf], failed: bool) -> String {
    match (removed.is_empty(), failed) {
        (_, true) => "failed".to_string(),
        (true, false) => "none".to_string(),
        (false, false) => format!("removed {}", shown_paths(removed)),
    }
}

struct PathEdit {
    path: PathBuf,
    next: String,
}

fn path_edits(home: &Path, spellings: &[String]) -> Result<Vec<PathEdit>> {
    #[cfg(windows)]
    {
        let _ = home;
        windows_plan(spellings)
    }
    #[cfg(not(windows))]
    {
        rc_plan(home, spellings)
    }
}

fn apply_path_edits(edits: &[PathEdit]) -> Result<Vec<PathBuf>> {
    #[cfg(windows)]
    {
        windows_apply(edits)
    }
    #[cfg(not(windows))]
    {
        let mut written = Vec::new();
        for edit in edits {
            fs::write(&edit.path, &edit.next)
                .with_context(|| format!("could not update {}", edit.path.display()))?;
            written.push(edit.path.clone());
        }
        Ok(written)
    }
}

fn path_plan_label(edits: &[PathEdit]) -> String {
    if edits.is_empty() {
        "unchanged".to_string()
    } else {
        let paths: Vec<PathBuf> = edits.iter().map(|edit| edit.path.clone()).collect();
        format!("edit {}", shown_paths(&paths))
    }
}

fn edited_label(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        "unchanged".to_string()
    } else {
        format!("edited {}", shown_paths(paths))
    }
}

#[cfg(not(windows))]
fn rc_plan(home: &Path, spellings: &[String]) -> Result<Vec<PathEdit>> {
    let mut edits = Vec::new();
    for (path, fish) in rc_files(home) {
        let Ok(text) = fs::read_to_string(&path) else {
            if path.exists() {
                bail!("could not read {}", path.display());
            }
            continue;
        };
        let next = strip_installer_blocks(&text, spellings, fish);
        if next != text {
            edits.push(PathEdit { path, next });
        }
    }
    Ok(edits)
}

#[cfg(not(windows))]
fn rc_files(home: &Path) -> Vec<(PathBuf, bool)> {
    vec![
        (home.join(".zshrc"), false),
        (home.join(".bashrc"), false),
        (home.join(".bash_profile"), false),
        (home.join(".config").join("fish").join("config.fish"), true),
    ]
}

#[cfg(any(not(windows), test))]
pub(crate) fn strip_installer_blocks(text: &str, dirs: &[String], fish: bool) -> String {
    let mut out = text.to_string();
    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        let line = if fish {
            format!("fish_add_path --prepend \"{dir}\"")
        } else {
            format!("export PATH=\"{dir}:$PATH\"")
        };
        out = strip_exact_block(&out, &line);
    }
    out
}

#[cfg(any(not(windows), test))]
fn strip_exact_block(text: &str, line: &str) -> String {
    let with_nl = format!("\n# chatkeep\n{line}\n");
    let at_eof = format!("\n# chatkeep\n{line}");
    let mut out = text.replace(&with_nl, "");
    if out.ends_with(&at_eof) {
        out.truncate(out.len() - at_eof.len());
    }
    out
}

pub(crate) fn dir_spellings(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    push_spelling(&mut out, dir.to_string_lossy().into_owned());
    if let Ok(canon) = dir.canonicalize() {
        push_spelling(&mut out, canon.to_string_lossy().into_owned());
    }
    out
}

fn push_spelling(out: &mut Vec<String>, text: String) {
    let text = text.trim_end_matches(['/', '\\']).to_string();
    if !text.is_empty() && !out.iter().any(|existing| existing == &text) {
        out.push(text);
    }
}

#[cfg(any(windows, test))]
pub(crate) fn without_install_dirs(path: &str, dirs: &[String]) -> String {
    let needles: Vec<String> = dirs
        .iter()
        .map(|dir| win_key(dir))
        .filter(|dir| !dir.is_empty())
        .collect();
    path.split(';')
        .filter(|part| {
            let key = win_key(part);
            !needles.iter().any(|needle| needle == &key)
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(any(windows, test))]
fn win_key(value: &str) -> String {
    value.trim().trim_end_matches('\\').to_ascii_lowercase()
}

fn refuse_cursor(path: &Path) -> Result<()> {
    if cursor_owned(path) {
        bail!(
            "refusing to delete {} because it is inside Cursor data",
            path.display()
        );
    }
    Ok(())
}

fn cursor_owned(path: &Path) -> bool {
    let path = normalize(path);
    if path.components().any(|part| part.as_os_str() == ".cursor") {
        return true;
    }
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".cursor"));
    }
    if let Ok(config) = crate::config::cursor_config_dir() {
        roots.push(config);
    }
    roots.iter().any(|root| path.starts_with(normalize(root)))
}

fn normalize(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path.to_path_buf()
    };
    abs.canonicalize().unwrap_or(abs)
}

#[cfg(windows)]
fn windows_plan(spellings: &[String]) -> Result<Vec<PathEdit>> {
    let Some(current) = read_user_path()? else {
        return Ok(Vec::new());
    };
    let next = without_install_dirs(&current, spellings);
    if next == current {
        return Ok(Vec::new());
    }
    Ok(vec![PathEdit {
        path: PathBuf::from("user PATH"),
        next,
    }])
}

#[cfg(windows)]
fn windows_apply(edits: &[PathEdit]) -> Result<Vec<PathBuf>> {
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    write_user_path(&edits[0].next)?;
    Ok(vec![PathBuf::from("user PATH")])
}

#[cfg(windows)]
fn read_user_path() -> Result<Option<String>> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RegQueryValueExW,
    };

    let key = open_env_key(KEY_READ | KEY_SET_VALUE)?;
    let _close = Key(key);
    for name in ["Path", "PATH"] {
        let wide = wide_null(name);
        let mut kind: REG_VALUE_TYPE = 0;
        let mut size: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                key,
                wide.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            continue;
        }
        if status != ERROR_SUCCESS {
            bail!("could not read the user PATH");
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            bail!("user PATH is not a string");
        }
        let mut buf = vec![0_u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                key,
                wide.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("could not read the user PATH");
        }
        buf.truncate(size as usize);
        return Ok(Some(utf16_bytes(&buf)));
    }
    Ok(None)
}

#[cfg(windows)]
fn write_user_path(value: &str) -> Result<()> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RegQueryValueExW,
        RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    let key = open_env_key(KEY_READ | KEY_SET_VALUE)?;
    let _close = Key(key);
    let mut chosen = "Path";
    let mut kind = REG_SZ;
    for name in ["Path", "PATH"] {
        let wide = wide_null(name);
        let mut found: REG_VALUE_TYPE = 0;
        let mut size: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                key,
                wide.as_ptr(),
                std::ptr::null(),
                &mut found,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_SUCCESS && (found == REG_SZ || found == REG_EXPAND_SZ) {
            chosen = name;
            kind = found;
            break;
        }
    }
    let wide_name = wide_null(chosen);
    let data = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let bytes = u32::try_from(data.len() * 2).unwrap_or(u32::MAX);
    let status = unsafe {
        RegSetValueExW(
            key,
            wide_name.as_ptr(),
            0,
            kind,
            data.as_ptr() as *const u8,
            bytes,
        )
    };
    if status != ERROR_SUCCESS {
        bail!("could not update the user PATH");
    }
    let notice = wide_null("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            notice.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn open_env_key(access: u32) -> Result<windows_sys::Win32::System::Registry::HKEY> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, RegOpenKeyExW};

    let name = wide_null("Environment");
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key) };
    if status != ERROR_SUCCESS {
        bail!("could not open the user environment");
    }
    Ok(key)
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn utf16_bytes(buf: &[u8]) -> String {
    let units: Vec<u16> = buf
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect();
    let end = units
        .iter()
        .rposition(|unit| *unit != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    String::from_utf16_lossy(&units[..end])
}

#[cfg(windows)]
struct Key(windows_sys::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for Key {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::Registry::RegCloseKey(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_the_installer_block() {
        let dir = "/tmp/chatkeep-bin";
        let text = format!(
            "export PATH=\"/usr/local/bin:$PATH\"\n# keep {dir}\n\n# chatkeep\nexport PATH=\"{dir}:$PATH\"\nalias ll='ls'\n"
        );
        let next = strip_installer_blocks(&text, &[dir.to_string()], false);
        assert_eq!(
            next,
            "export PATH=\"/usr/local/bin:$PATH\"\n# keep /tmp/chatkeep-bin\nalias ll='ls'\n"
        );
        let fish =
            format!("\n# chatkeep\nfish_add_path --prepend \"{dir}\"\nset -x PATH /usr/bin\n");
        assert_eq!(
            strip_installer_blocks(&fish, &[dir.to_string()], true),
            "set -x PATH /usr/bin\n"
        );
        assert_eq!(
            strip_installer_blocks(&text, &[dir.to_string()], true),
            text
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_links_to_the_managed_binary_are_removed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let managed = home.join(".chatkeep").join("bin");
        fs::create_dir_all(&managed).unwrap();
        let binary = managed.join("chatkeep");
        fs::write(&binary, "bin").unwrap();
        let elsewhere = home.join("other").join("chatkeep");
        fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        fs::write(&elsewhere, "other").unwrap();
        let dirs: Vec<PathBuf> = ["local", "user", "file", "other-link", "dangling", "empty"]
            .iter()
            .map(|name| {
                let dir = home.join("dirs").join(name);
                fs::create_dir_all(&dir).unwrap();
                dir
            })
            .collect();
        symlink(&binary, dirs[0].join("chatkeep")).unwrap();
        symlink(
            Path::new("../../.chatkeep/bin/chatkeep"),
            dirs[1].join("chatkeep"),
        )
        .unwrap();
        fs::write(dirs[2].join("chatkeep"), "regular file").unwrap();
        symlink(&elsewhere, dirs[3].join("chatkeep")).unwrap();
        let spellings = dir_spellings(&managed);
        let found = managed_links(&dirs, &binary, &spellings);
        assert_eq!(found, [dirs[0].join("chatkeep"), dirs[1].join("chatkeep")]);

        fs::remove_file(&binary).unwrap();
        symlink(&binary, dirs[4].join("chatkeep")).unwrap();
        let found = managed_links(&dirs, &binary, &spellings);
        assert_eq!(
            found,
            [
                dirs[0].join("chatkeep"),
                dirs[1].join("chatkeep"),
                dirs[4].join("chatkeep")
            ]
        );
        let (removed, error) = remove_links(&found);
        assert!(error.is_none());
        assert_eq!(removed, found);
        for link in &found {
            assert!(fs::symlink_metadata(link).is_err(), "{}", link.display());
        }
        assert_eq!(
            fs::read_to_string(dirs[2].join("chatkeep")).unwrap(),
            "regular file"
        );
        assert_eq!(fs::read_link(dirs[3].join("chatkeep")).unwrap(), elsewhere);
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "other");
        assert!(managed_links(&dirs, &binary, &spellings).is_empty());
        assert_eq!(link_plan_label(&[]), "none");
        assert_eq!(linked_label(&[], false), "none");
        assert_eq!(linked_label(&removed, true), "failed");
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_lists_links_and_removes_nothing() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let managed = home.join(".chatkeep").join("bin");
        fs::create_dir_all(&managed).unwrap();
        let binary = managed.join("chatkeep");
        fs::write(&binary, "bin").unwrap();
        let local = home.join(".local").join("bin");
        fs::create_dir_all(&local).unwrap();
        symlink(&binary, local.join("chatkeep")).unwrap();
        let state = home.join("state");
        let args = UninstallArgs {
            dry_run: true,
            yes: false,
            purge: false,
        };
        execute(&args, home, &binary, &state, &dir_spellings(&managed)).unwrap();
        assert!(fs::symlink_metadata(local.join("chatkeep")).is_ok());
        assert!(binary.exists());
        let links = managed_links(&link_dirs(home), &binary, &dir_spellings(&managed));
        assert_eq!(links, [local.join("chatkeep")]);
        assert!(link_plan_label(&links).starts_with("remove "));
    }

    #[test]
    fn windows_path_drops_only_the_install_dir() {
        let dir = r"C:\Users\me\.chatkeep\bin";
        let path = r"C:\Users\me\.chatkeep\bin;C:\Windows;C:\Users\me\.chatkeep\bin\extra";
        assert_eq!(
            without_install_dirs(path, &[dir.to_string()]),
            r"C:\Windows;C:\Users\me\.chatkeep\bin\extra"
        );
        assert_eq!(
            without_install_dirs(path, &[format!("{dir}\\")]),
            r"C:\Windows;C:\Users\me\.chatkeep\bin\extra"
        );
        assert_eq!(
            without_install_dirs("A;B;", &[r"C:\missing".to_string()]),
            "A;B;"
        );
    }
}
