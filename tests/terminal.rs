//! Drives `chatkeep` inside a real pseudo-terminal (via `script`) to check what a person sees when
//! a picker is interrupted with Ctrl-C. Only a dry run against a scratch HOME runs: `rx -n` with
//! no target stops at its workspace picker, nothing is selected or confirmed, and a dry run
//! never writes Cursor data.

#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;

const HIDE_CURSOR: &str = "\u{1b}[?25l";
const SHOW_CURSOR: &str = "\u{1b}[?25h";

fn chatkeep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chatkeep"))
}

fn has_script() -> bool {
    Command::new("script")
        .arg("-q")
        .arg("/dev/null")
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Runs `shell` (a `sh -c` script) inside a pseudo-terminal 80 columns wide.
fn in_pty(shell: &str, home: &Path) -> Child {
    let shell = format!("stty cols 80 rows 40; {shell}");
    let mut command = Command::new("script");
    if cfg!(target_os = "macos") {
        command.args(["-q", "/dev/null", "sh", "-c", &shell]);
    } else {
        command.args(["-qec", &format!("sh -c '{shell}'"), "/dev/null"]);
    }
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("CHATKEEP_HOME", home.join("chatkeep-state"))
        .env("CHATKEEP_NO_UPDATE_CHECK", "1")
        .env("CHATKEEP_NO_INDEX", "1")
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .env_remove("NO_COLOR")
        .env_remove("LC_ALL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script runs")
}

fn collect(child: &mut Child) -> Arc<Mutex<String>> {
    let out = Arc::new(Mutex::new(String::new()));
    let mut stdout = child.stdout.take().unwrap();
    let sink = Arc::clone(&out);
    std::thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        while let Ok(read) = stdout.read(&mut buf) {
            if read == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..read]));
        }
    });
    out
}

fn wait_for(out: &Arc<Mutex<String>>, needle: &str) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if out.lock().unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn finish(mut child: Child) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("chatkeep did not exit after Ctrl-C");
}

/// A scratch HOME with one fake Cursor installation (`~/.cursor-scratch`) and one workspace.
fn scratch_home() -> (tempfile::TempDir, PathBuf) {
    disable_update_check();
    let home = tempfile::tempdir().unwrap();
    let root = home.path().to_path_buf();
    let layout = install_layout(&root, "scratch", root.join(".cursor-scratch"));
    let project = root.join("projects").join("app");
    std::fs::create_dir_all(&project).unwrap();
    write_folder_workspace(&layout, "0123456789abcdef0123456789abcdef", &project);
    let global = layout.global_db();
    (home, global)
}

#[test]
fn ctrl_c_in_a_picker_gives_the_cursor_back() {
    if !has_script() {
        eprintln!("skipped: no script(1)");
        return;
    }
    let (home, global) = scratch_home();
    let before = std::fs::read(&global).unwrap();
    let chatkeep = chatkeep_bin();
    let mut child = in_pty(
        &format!(
            "trap true INT; {} cursor rx -n; echo; echo after-chatkeep; stty -a",
            chatkeep.display()
        ),
        home.path(),
    );
    let out = collect(&mut child);
    assert!(
        wait_for(&out, "Reindex which workspace"),
        "picker never appeared: {:?}",
        out.lock().unwrap()
    );
    assert!(wait_for(&out, HIDE_CURSOR), "the picker hides the cursor");
    std::thread::sleep(Duration::from_millis(200));
    child.stdin.as_mut().unwrap().write_all(b"\x03").unwrap();
    assert!(
        wait_for(&out, "after-chatkeep"),
        "{:?}",
        out.lock().unwrap()
    );
    std::thread::sleep(Duration::from_millis(300));
    finish(child);
    let text = out.lock().unwrap().clone();
    let hidden = text.rfind(HIDE_CURSOR).unwrap();
    assert!(
        text[hidden..].contains(SHOW_CURSOR),
        "cursor left hidden after Ctrl-C: {:?}",
        &text[hidden..]
    );
    let state = &text[text.rfind("after-chatkeep").unwrap()..];
    let words: Vec<&str> = state.split_whitespace().collect();
    assert!(
        words.contains(&"echo") && !words.contains(&"-echo"),
        "terminal echo left off after Ctrl-C: {state}"
    );
    assert_eq!(
        std::fs::read(&global).unwrap(),
        before,
        "the dry run changed nothing"
    );
    assert!(
        !home.path().join("chatkeep-state").join("backups").exists(),
        "no write session was started"
    );
}
