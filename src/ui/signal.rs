//! Ctrl-C cleanup: put the terminal back the way the prompts and spinners found it.
//!
//! dialoguer hides the cursor and console switches off echo while a prompt reads a key or a
//! password; the default SIGINT disposition kills the process before either is undone, which
//! leaves the shell with no cursor or no echo. Spinner lines would also stay on screen. The
//! handler below only uses async-signal-safe calls, undoes what is active, then re-raises
//! SIGINT with the default disposition so the exit status still says "killed by SIGINT".
//!
//! Writes are unaffected: the process still dies from SIGINT at the same point it did without
//! the handler, so an open global-DB transaction is never committed and the journal is left
//! exactly as before for the next run to find.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static PROMPTS: AtomicUsize = AtomicUsize::new(0);
static LINES: AtomicUsize = AtomicUsize::new(0);

/// Marks a prompt as active for the lifetime of the guard.
pub struct PromptGuard(());

impl PromptGuard {
    pub fn new() -> Self {
        PROMPTS.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Default for PromptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        PROMPTS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Counts terminal lines a spinner or progress bar is drawing, so Ctrl-C can erase them.
pub struct LinesGuard(usize);

impl LinesGuard {
    pub fn new(lines: usize) -> Self {
        LINES.fetch_add(lines, Ordering::SeqCst);
        Self(lines)
    }
}

impl Drop for LinesGuard {
    fn drop(&mut self) {
        LINES.fetch_sub(self.0, Ordering::SeqCst);
    }
}

pub fn prompts_active() -> usize {
    PROMPTS.load(Ordering::SeqCst)
}

pub fn lines_active() -> usize {
    LINES.load(Ordering::SeqCst)
}

/// The bytes the handler writes to stderr for a given state. Kept pure so it can be tested.
pub fn cleanup_sequence(lines: usize, prompts: usize) -> String {
    let mut out = String::new();
    if lines > 0 {
        out.push_str("\r\u{1b}[2K");
        for _ in 1..lines {
            out.push_str("\u{1b}[1A\u{1b}[2K");
        }
    }
    if prompts > 0 {
        out.push_str("\u{1b}[?25h\r\n");
    }
    out
}

#[cfg(unix)]
mod imp {
    use super::{INSTALLED, LINES, PROMPTS};
    use std::ffi::{c_int, c_void};
    use std::io::IsTerminal;
    use std::sync::atomic::{AtomicBool, Ordering};

    const SIGINT: c_int = 2;
    const SIG_DFL: usize = 0;
    const SIG_IGN: usize = 1;
    const TCSANOW: c_int = 0;
    const STDIN: c_int = 0;
    const STDERR: c_int = 2;

    /// Opaque, generously sized and aligned storage for `struct termios`
    /// (60 bytes on Linux, 72 on macOS).
    #[repr(C, align(8))]
    struct Termios([u8; 256]);

    static mut SAVED: Termios = Termios([0; 256]);
    static HAVE_SAVED: AtomicBool = AtomicBool::new(false);
    static TTY: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn signal(signum: c_int, handler: usize) -> usize;
        fn raise(signum: c_int) -> c_int;
        fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
        fn tcgetattr(fd: c_int, termios: *mut c_void) -> c_int;
        fn tcsetattr(fd: c_int, action: c_int, termios: *const c_void) -> c_int;
    }

    fn put(bytes: &[u8]) {
        // SAFETY: write(2) is async-signal-safe; the buffer outlives the call.
        unsafe {
            write(STDERR, bytes.as_ptr().cast(), bytes.len());
        }
    }

    extern "C" fn on_interrupt(_: c_int) {
        if TTY.load(Ordering::SeqCst) {
            let lines = LINES.load(Ordering::SeqCst);
            let prompts = PROMPTS.load(Ordering::SeqCst);
            if lines > 0 {
                put(b"\r\x1b[2K");
                for _ in 1..lines {
                    put(b"\x1b[1A\x1b[2K");
                }
            }
            if prompts > 0 {
                if HAVE_SAVED.load(Ordering::SeqCst) {
                    // SAFETY: SAVED was filled by tcgetattr before the handler was installed
                    // and is never written again; tcsetattr is async-signal-safe.
                    unsafe {
                        tcsetattr(STDIN, TCSANOW, (&raw const SAVED).cast());
                    }
                }
                put(b"\x1b[?25h\r\n");
            }
        }
        // SAFETY: both calls are async-signal-safe. SIGINT is blocked while this handler runs,
        // so the re-raised signal is delivered with the default action once it returns.
        unsafe {
            signal(SIGINT, SIG_DFL);
            raise(SIGINT);
        }
    }

    pub fn install() {
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        TTY.store(std::io::stderr().is_terminal(), Ordering::SeqCst);
        if std::io::stdin().is_terminal() {
            // SAFETY: SAVED is only written here, once, before the handler can run.
            let ok = unsafe { tcgetattr(STDIN, (&raw mut SAVED).cast()) } == 0;
            HAVE_SAVED.store(ok, Ordering::SeqCst);
        }
        // SAFETY: the handler only touches atomics and async-signal-safe libc calls.
        let previous = unsafe { signal(SIGINT, on_interrupt as extern "C" fn(c_int) as usize) };
        if previous == SIG_IGN {
            // A parent asked for SIGINT to be ignored (nohup, background jobs): keep it that way.
            // SAFETY: restoring the inherited disposition.
            unsafe {
                signal(SIGINT, SIG_IGN);
            }
        }
    }
}

#[cfg(not(unix))]
mod imp {
    pub fn install() {
        super::INSTALLED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Installs the Ctrl-C handler once. Child processes get the default disposition back on exec.
pub fn install() {
    imp::install();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_count_and_release() {
        let base_prompts = prompts_active();
        let base_lines = lines_active();
        {
            let _prompt = PromptGuard::new();
            let _lines = LinesGuard::new(3);
            assert!(prompts_active() > base_prompts);
            assert!(lines_active() >= base_lines + 3);
        }
        assert_eq!(prompts_active(), base_prompts);
    }

    #[test]
    fn cleanup_restores_the_cursor_and_erases_spinner_lines() {
        assert_eq!(cleanup_sequence(0, 0), "");
        assert_eq!(cleanup_sequence(1, 0), "\r\u{1b}[2K");
        assert_eq!(
            cleanup_sequence(3, 0),
            "\r\u{1b}[2K\u{1b}[1A\u{1b}[2K\u{1b}[1A\u{1b}[2K"
        );
        let prompt = cleanup_sequence(0, 1);
        assert!(prompt.contains("\u{1b}[?25h"), "shows the cursor");
        assert!(prompt.ends_with("\r\n"), "leaves the shell on a fresh line");
    }
}
