//! The folder name Claude Code gives a project in `~/.claude/projects`.
//!
//! Claude Code works on JavaScript strings, so the rules below count UTF-16 code units: a
//! character outside the Basic Multilingual Plane becomes two dashes, and the hash runs over
//! code units, not bytes or characters.

/// Longest name Claude Code keeps before it cuts the name and appends a hash.
const MAX_LEN: usize = 200;

/// Every code unit outside `[A-Za-z0-9]` becomes `-`. A name longer than 200 units keeps its
/// first 200 plus `-` and the base-36 hash of the full path.
pub fn project_slug(path: &str) -> String {
    let units: Vec<u16> = path.encode_utf16().collect();
    let slug: String = units
        .iter()
        .map(|&unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect();
    if slug.len() <= MAX_LEN {
        return slug;
    }
    format!(
        "{}-{}",
        &slug[..MAX_LEN],
        base36(string_hash(&units).unsigned_abs())
    )
}

/// Java's `String.hashCode` on UTF-16 code units, as Claude Code computes it.
fn string_hash(units: &[u16]) -> i32 {
    units.iter().fold(0i32, |hash, &unit| {
        hash.wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(i32::from(unit))
    })
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("base-36 digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths_replace_every_non_alphanumeric_unit() {
        assert_eq!(
            project_slug("/Users/me/projects/my.app_v2"),
            "-Users-me-projects-my-app-v2"
        );
        assert_eq!(project_slug(r"C:\Users\me\app"), "C--Users-me-app");
        assert_eq!(project_slug("/Users/me/café/x"), "-Users-me-caf--x");
    }

    #[test]
    fn characters_outside_the_bmp_become_two_dashes() {
        assert_eq!(project_slug("/a/\u{1F600}/b"), "-a----b");
    }

    /// Reference values from Claude Code's own function, run in Node.
    #[test]
    fn long_paths_are_cut_and_hashed_like_claude_code() {
        let path = format!("/Users/me/{}project", "deep/".repeat(45));
        let slug = project_slug(&path);
        assert_eq!(slug, format!("-Users-me-{}-sgtoh5", "deep-".repeat(38)));
        assert_eq!(slug.len(), MAX_LEN + "-sgtoh5".len());
    }

    #[test]
    fn hash_matches_javascript_for_known_inputs() {
        let units: Vec<u16> = "/Users/me/café/x".encode_utf16().collect();
        assert_eq!(base36(string_hash(&units).unsigned_abs()), "y4mbqj");
        assert_eq!(base36(0), "0");
        assert_eq!(base36(i32::MIN.unsigned_abs()), "zik0zk");
    }
}
