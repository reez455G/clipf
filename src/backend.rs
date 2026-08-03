//! Clipboard backends: the local helper tools, plus OSC 52 as the fallback
//! that works over SSH.

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::exit::ClipfError;
use crate::secret::Secret;
use crate::term;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Osc52,
    Xclip,
    Xsel,
    WlCopy,
    PbCopy,
    ClipExe,
    Termux,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::detect()),
            "osc52" | "osc" => Some(Backend::Osc52),
            "xclip" => Some(Backend::Xclip),
            "xsel" => Some(Backend::Xsel),
            "wl" | "wl-copy" | "wayland" => Some(Backend::WlCopy),
            "pbcopy" | "macos" => Some(Backend::PbCopy),
            "clip.exe" | "clip" | "wsl" => Some(Backend::ClipExe),
            "termux" | "termux-clipboard" | "android" => Some(Backend::Termux),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Osc52 => "osc52",
            Backend::Xclip => "xclip",
            Backend::Xsel => "xsel",
            Backend::WlCopy => "wl",
            Backend::PbCopy => "pbcopy",
            Backend::ClipExe => "clip.exe",
            Backend::Termux => "termux",
        }
    }

    /// The external command this backend shells out to, if any.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Backend::Osc52 => None,
            Backend::Xclip => Some("xclip"),
            Backend::Xsel => Some("xsel"),
            Backend::WlCopy => Some("wl-copy"),
            Backend::PbCopy => Some("pbcopy"),
            Backend::ClipExe => Some("clip.exe"),
            Backend::Termux => Some("termux-clipboard-set"),
        }
    }

    /// Pick a backend for this environment. OSC 52 is the last resort because
    /// it is the only one with a size limit, but on a headless server it is
    /// also the only one that reaches the user's actual clipboard.
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") && which("pbcopy").is_some() {
            Backend::PbCopy
        } else if (cfg!(windows) || term::is_wsl()) && which("clip.exe").is_some() {
            Backend::ClipExe
        } else if which("termux-clipboard-set").is_some() {
            Backend::Termux
        } else if env::var_os("WAYLAND_DISPLAY").is_some() && which("wl-copy").is_some() {
            Backend::WlCopy
        } else if env::var_os("DISPLAY").is_some() && which("xclip").is_some() {
            Backend::Xclip
        } else if env::var_os("DISPLAY").is_some() && which("xsel").is_some() {
            Backend::Xsel
        } else {
            Backend::Osc52
        }
    }

    fn copy_args(self) -> &'static [&'static str] {
        match self {
            // Default to CLIPBOARD, not PRIMARY. Forgetting this is the single
            // most common reason "xclip didn't work" - PRIMARY only pastes via
            // middle-click, not Ctrl+V.
            Backend::Xclip => &["-selection", "clipboard"],
            Backend::Xsel => &["--input", "--clipboard"],
            _ => &[],
        }
    }

    fn paste_command(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Backend::Xclip => Some(("xclip", &["-selection", "clipboard", "-o"])),
            Backend::Xsel => Some(("xsel", &["--output", "--clipboard"])),
            Backend::WlCopy => Some(("wl-paste", &["-n"])),
            Backend::PbCopy => Some(("pbpaste", &[])),
            Backend::ClipExe => Some((
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw",
                ],
            )),
            Backend::Termux => Some(("termux-clipboard-get", &[])),
            Backend::Osc52 => None,
        }
    }

    /// Pipe `data` into the local helper tool.
    pub fn copy_local(self, data: &[u8]) -> Result<(), ClipfError> {
        let cmd = self
            .command()
            .ok_or_else(|| ClipfError::backend_unavailable("osc52 is not an external command"))?;

        // clip.exe decodes stdin with the legacy OEM code page, which mangles any
        // non-ASCII UTF-8. It does honour a UTF-16LE BOM, so transcode text payloads.
        // Binary payloads (invalid UTF-8) are passed through untouched.
        let transcoded: Secret;
        let payload: &[u8] = if self == Backend::ClipExe {
            match std::str::from_utf8(data) {
                Ok(s) if !s.is_ascii() => {
                    transcoded = Secret::from_vec(utf16le_with_bom(s));
                    &transcoded
                }
                _ => data,
            }
        } else {
            data
        };

        let mut child = Command::new(cmd)
            .args(self.copy_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| spawn_error(cmd, e))?;

        // Take stdin and drop it so the child sees EOF; xclip and wl-copy both
        // wait for the stream to close before daemonising.
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| ClipfError::backend_failed(format!("{cmd}: no stdin pipe")))?;
            stdin
                .write_all(payload)
                .map_err(|e| ClipfError::backend_failed(format!("writing to {cmd}: {e}")))?;
        }

        let status = child
            .wait()
            .map_err(|e| ClipfError::backend_failed(format!("waiting for {cmd}: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(exit_error(cmd, status))
        }
    }

    pub fn paste(self) -> Result<Vec<u8>, ClipfError> {
        let (cmd, args) = self.paste_command().ok_or_else(|| {
            ClipfError::paste_unsupported(
                "reading the clipboard over OSC 52 is not supported by terminals \
                 (the query form is disabled almost everywhere as a security measure)",
            )
        })?;

        let out = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| spawn_error(cmd, e))?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(exit_error(cmd, out.status))
        }
    }
}

/// `which(1)` without spawning anything.
pub fn which(cmd: &str) -> Option<PathBuf> {
    if cmd.contains('/') || cmd.contains('\\') {
        let p = PathBuf::from(cmd);
        return is_executable(&p).then_some(p);
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(cmd))
        .find(|p| is_executable(p))
}

/// A spawn failure: the helper binary could not be started at all, even
/// though `which` found it moments earlier (a TOCTOU race, a permission
/// problem, or a broken symlink). An agent should treat this as "the
/// backend is not actually usable right now."
fn spawn_error(cmd: &str, e: std::io::Error) -> ClipfError {
    ClipfError::backend_unavailable(format!("cannot run {cmd}: {e}"))
}

/// The helper ran but exited non-zero. Distinct from `spawn_error`: the
/// backend is present and was tried, it just failed.
fn exit_error(cmd: &str, status: std::process::ExitStatus) -> ClipfError {
    ClipfError::backend_failed(format!("{cmd} exited with {status}"))
}

/// UTF-16LE with a byte-order mark. clip.exe detects the BOM and decodes the
/// rest as UTF-16, which is the only encoding it handles losslessly.
fn utf16le_with_bom(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + s.len() * 2);
    v.extend_from_slice(&[0xff, 0xfe]);
    for unit in s.encode_utf16() {
        v.extend_from_slice(&unit.to_le_bytes());
    }
    v
}

fn is_executable(p: &std::path::Path) -> bool {
    // Checking the mode bits is enough here: this only decides which helper to
    // try, and a genuine exec failure is reported by copy_local anyway.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_names_and_aliases() {
        assert_eq!(Backend::parse("osc52"), Some(Backend::Osc52));
        assert_eq!(Backend::parse("OSC52"), Some(Backend::Osc52));
        assert_eq!(Backend::parse("wl-copy"), Some(Backend::WlCopy));
        assert_eq!(Backend::parse("wayland"), Some(Backend::WlCopy));
        assert_eq!(Backend::parse("clip"), Some(Backend::ClipExe));
        assert_eq!(Backend::parse("termux"), Some(Backend::Termux));
        assert_eq!(Backend::parse("nonsense"), None);
    }

    #[test]
    fn names_round_trip_through_parse() {
        for b in [
            Backend::Osc52,
            Backend::Xclip,
            Backend::Xsel,
            Backend::WlCopy,
            Backend::PbCopy,
            Backend::ClipExe,
            Backend::Termux,
        ] {
            assert_eq!(Backend::parse(b.name()), Some(b), "{}", b.name());
        }
    }

    #[test]
    fn x11_backends_target_the_clipboard_selection() {
        assert!(Backend::Xclip.copy_args().contains(&"clipboard"));
        assert!(Backend::Xsel.copy_args().contains(&"--clipboard"));
    }

    #[test]
    fn osc52_has_no_local_command_and_cannot_be_pasted() {
        assert!(Backend::Osc52.command().is_none());
        // Osc52 is routed around copy_local/paste in main.rs (see
        // copy_osc52); calling them directly is a programming misuse, still
        // mapped to a defined exit code rather than panicking.
        assert_eq!(
            Backend::Osc52.copy_local(b"x").unwrap_err().code(),
            5, // BackendUnavailable: osc52 has no local helper to run
        );
        assert_eq!(
            Backend::Osc52.paste().unwrap_err().code(),
            8, // PasteUnsupported: terminals disable the OSC 52 query form
        );
    }

    #[test]
    fn spawn_failure_is_backend_unavailable() {
        let e = spawn_error("nope", std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(e.code(), 5);
        assert!(e.to_string().contains("nope"));
    }

    #[test]
    fn nonzero_exit_is_backend_failed() {
        // Built directly rather than by spawning a real process, so this
        // stays deterministic and needs nothing installed on the runner.
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(1 << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(1)
        };
        let e = exit_error("xclip", status);
        assert_eq!(e.code(), 6);
        assert!(e.to_string().contains("xclip"));
    }


    #[test]
    fn which_finds_a_standard_binary_and_rejects_nonsense() {
        #[cfg(unix)]
        assert!(which("sh").is_some());
        #[cfg(windows)]
        assert!(which("cmd.exe").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn utf16le_bom_encodes_non_ascii() {
        assert_eq!(utf16le_with_bom("é"), vec![0xff, 0xfe, 0xe9, 0x00]);
        assert_eq!(utf16le_with_bom("A"), vec![0xff, 0xfe, 0x41, 0x00]);
    }

    #[test]
    fn utf16le_bom_encodes_surrogate_pairs() {
        // U+1F600, outside the BMP, must become a surrogate pair.
        assert_eq!(
            utf16le_with_bom("\u{1f600}"),
            vec![0xff, 0xfe, 0x3d, 0xd8, 0x00, 0xde]
        );
    }
}
