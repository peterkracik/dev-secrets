//! Copying text to the system clipboard.
//!
//! To avoid heavy, platform-specific dependencies we try the common
//! command-line clipboard tools first, then fall back to the terminal's
//! OSC 52 escape sequence (which also works over SSH / tmux). The returned
//! string names the method used so the UI can report it.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// Copy `text` to the clipboard. Returns a short description of the mechanism
/// that succeeded (e.g. "pbcopy", "wl-copy", "terminal (OSC 52)").
pub fn copy(text: &str) -> Result<&'static str> {
    for (program, args, label) in candidates() {
        if try_command(program, args, text) {
            return Ok(label);
        }
    }
    // Fall back to OSC 52, which targets the terminal emulator directly.
    osc52(text)?;
    Ok("terminal (OSC 52)")
}

/// Clipboard helper commands to try, in order, for the current platform.
fn candidates() -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    if cfg!(target_os = "macos") {
        vec![("pbcopy", &[][..], "pbcopy")]
    } else if cfg!(target_os = "windows") {
        vec![("clip", &[][..], "clip")]
    } else {
        let mut v: Vec<(&str, &[&str], &str)> = Vec::new();
        // Prefer Wayland when present, then X11 tools.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            v.push(("wl-copy", &[][..], "wl-copy"));
        }
        v.push(("xclip", &["-selection", "clipboard"][..], "xclip"));
        v.push(("xsel", &["--clipboard", "--input"][..], "xsel"));
        v
    }
}

/// Spawn `program`, pipe `text` to its stdin, and report whether it succeeded.
fn try_command(program: &str, args: &[&str], text: &str) -> bool {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false; // tool not installed
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    matches!(child.wait(), Ok(status) if status.success())
}

/// Emit an OSC 52 clipboard-set sequence to the terminal on stdout.
fn osc52(text: &str) -> Result<()> {
    let encoded = base64_encode(text.as_bytes());
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{encoded}\x07")?;
    out.flush()?;
    Ok(())
}

/// Minimal standard base64 encoder (no external crate needed).
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_utf8() {
        assert_eq!(base64_encode("héllo".as_bytes()), "aMOpbGxv");
    }
}
