//! chatkeep: keep AI coding chat history attached to your projects.
//!
//! This tool is not affiliated with or endorsed by Anysphere, Inc. (Cursor).
//! It accesses locally stored data on your machine for personal use.
//! See DISCLAIMER.md for details.

fn main() {
    std::process::exit(chatkeep::cli::run());
}
